//! Звуки для кнопок (как в Soundpad): чтение файла и подготовка к микшеру.
//!
//! Файл декодируется целиком один раз, сводится в стерео float32 и приводится
//! к частоте микрофона. Короткие звуки так проигрываются без задержки.

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Расширения, которые понимает декодер.
pub const EXTENSIONS: &[&str] = &["mp3", "wav", "ogg", "flac", "m4a", "aac", "mp4", "mka", "mkv", "webm"];

/// Дольше не берём: 10 минут стерео при 48 кГц — это уже ~230 МБ памяти.
const MAX_SECONDS: usize = 600;

/// Читает файл и возвращает стерео-сэмплы (L, R, ...) в частоте `rate`.
pub fn load(path: &Path, rate: u32) -> Result<Vec<f32>, String> {
    let (src_rate, stereo) = decode_stereo(path)?;
    Ok(resample(&stereo, src_rate, rate))
}

fn decode_stereo(path: &Path) -> Result<(u32, Vec<f32>), String> {
    let file = File::open(path).map_err(|e| format!("не открыть файл: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|_| "формат файла не поддерживается".to_string())?;
    let track = format.default_track(TrackType::Audio).ok_or("в файле нет звука")?;
    let track_id = track.id;
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or("в файле нет звука")?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|_| "кодек не поддерживается".to_string())?;

    let mut rate = 0u32;
    let mut out: Vec<f32> = Vec::new();
    let mut tmp: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(Error::IoError(_)) => break, // конец файла у некоторых форматов
            Err(e) => return Err(format!("ошибка чтения: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            Err(Error::DecodeError(_)) => continue, // битый кадр — пропускаем
            Err(e) => return Err(format!("ошибка декодирования: {e}")),
        };
        let channels = buf.spec().channels().count().max(1);
        rate = buf.spec().rate();
        tmp.resize(buf.samples_interleaved(), 0.0);
        buf.copy_to_slice_interleaved(&mut tmp);
        for frame in tmp.chunks(channels) {
            // Моно → в оба канала; больше двух каналов → берём левый и правый.
            let l = frame[0];
            let r = if channels > 1 { frame[1] } else { l };
            out.push(l);
            out.push(r);
        }
        if rate > 0 && out.len() / 2 > rate as usize * MAX_SECONDS {
            return Err(format!("файл длиннее {} минут", MAX_SECONDS / 60));
        }
    }
    if out.is_empty() || rate == 0 {
        return Err("в файле нет звука".into());
    }
    Ok((rate, out))
}

/// Линейная интерполяция стерео-сигнала из `from` Гц в `to` Гц.
/// Для звуковых эффектов этого достаточно; одинаковые частоты — без изменений.
pub fn resample(stereo: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || from == 0 || to == 0 {
        return stereo.to_vec();
    }
    let frames_in = stereo.len() / 2;
    let frames_out = (frames_in as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    let mut out = Vec::with_capacity(frames_out * 2);
    for i in 0..frames_out {
        let pos = i as f64 * step;
        let i0 = pos as usize;
        let i1 = (i0 + 1).min(frames_in - 1);
        let t = (pos - i0 as f64) as f32;
        for c in 0..2 {
            let a = stereo[i0 * 2 + c];
            let b = stereo[i1 * 2 + c];
            out.push(a + (b - a) * t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_keeps_length_ratio_and_values() {
        let input: Vec<f32> = (0..441).flat_map(|i| [i as f32, -(i as f32)]).collect();
        let out = resample(&input, 44_100, 48_000);
        assert_eq!(out.len() / 2, 480);
        assert_eq!(out[0], 0.0);
        // Середина: кадр 240 на 48 кГц ≈ кадр 220.5 на 44.1 кГц.
        assert!((out[480] - 220.5).abs() < 0.01);
        assert!((out[481] + 220.5).abs() < 0.01);
    }

    #[test]
    fn resample_same_rate_is_identity() {
        let input = vec![0.1, 0.2, 0.3, 0.4];
        assert_eq!(resample(&input, 48_000, 48_000), input);
    }
}
