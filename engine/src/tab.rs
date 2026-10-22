//! Приём звука вкладки браузера от расширения MicMix (extension/).
//!
//! Расширение захватывает звук вкладки (chrome.tabCapture) и шлёт его сюда по
//! WebSocket на 127.0.0.1:PORT: сначала текстовое сообщение `{"rate":48000}`,
//! потом двоичные сообщения — float32 стерео (L, R, ...), little-endian.
//!
//! Подключаться к локальному порту может любая открытая страница, поэтому
//! принимаем только расширения (Origin chrome-extension:// / moz-extension://) —
//! иначе сайт мог бы подмешать свой звук в микрофон.

use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::StatusCode;
use tungstenite::Message;

use crate::engine::Feed;
use crate::ring::CHANNELS;
use crate::sounds::resample;

pub const PORT: u16 = 47_823;

/// Запускает сервер в отдельном потоке. Ошибка — порт занят (второй MicMix и т. п.).
pub fn start(feed: Arc<Feed>) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", PORT)).map_err(|e| format!("порт {PORT}: {e}"))?;
    std::thread::Builder::new()
        .name("tab-server".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                // Одна вкладка за раз: новое подключение ждёт, пока закроется прежнее.
                if let Err(e) = serve(stream, &feed) {
                    eprintln!("[tab] {e}");
                }
                feed.connected.store(false, Ordering::Release);
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn is_extension_origin(origin: &str) -> bool {
    origin.starts_with("chrome-extension://") || origin.starts_with("moz-extension://")
}

fn serve(stream: TcpStream, feed: &Feed) -> Result<(), String> {
    let check = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        let origin = req.headers().get("origin").and_then(|v| v.to_str().ok()).unwrap_or("");
        if is_extension_origin(origin) {
            Ok(resp)
        } else {
            let mut err = ErrorResponse::new(Some("only the MicMix extension may connect".into()));
            *err.status_mut() = StatusCode::FORBIDDEN;
            Err(err)
        }
    };
    let mut ws = tungstenite::accept_hdr(stream, check).map_err(|e| e.to_string())?;
    feed.connected.store(true, Ordering::Release);

    let mut src_rate = 48_000u32;
    let mut samples: Vec<f32> = Vec::new();
    loop {
        match ws.read().map_err(|e| e.to_string())? {
            Message::Text(t) => {
                // {"rate":44100} — частота AudioContext в расширении.
                if let Some(v) = t.split("\"rate\"").nth(1) {
                    let digits: String = v.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(r) = digits.parse::<u32>() {
                        if (8_000..=192_000).contains(&r) {
                            src_rate = r;
                        }
                    }
                }
            }
            Message::Binary(b) => {
                samples.clear();
                samples.extend(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])));
                samples.truncate(samples.len() / CHANNELS * CHANNELS);
                let rate = feed.rate.load(Ordering::Acquire);
                let data = resample(&samples, src_rate, rate);
                let mut q = feed.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                q.extend(data);
                let max = (rate as usize / 2) * CHANNELS; // не больше 0.5 с
                while q.len() > max {
                    q.pop_front();
                }
            }
            Message::Close(_) => return Ok(()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_extensions_may_connect() {
        assert!(is_extension_origin("chrome-extension://abcdef"));
        assert!(is_extension_origin("moz-extension://123"));
        assert!(!is_extension_origin("https://evil.example"));
        assert!(!is_extension_origin(""));
    }
}
