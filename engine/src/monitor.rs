//! Прослушивание звуков самому: копия звуков из кнопок уходит на устройство
//! вывода по умолчанию (наушники/колонки), как «слышать себя» в Soundpad.
//!
//! Звук приложений сюда не идёт — его и так слышно из самих приложений.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::ring::CHANNELS;

pub struct Monitor {
    queue: Arc<Mutex<VecDeque<f32>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    rate: u32,
}

impl Monitor {
    pub fn start(rate: u32) -> Monitor {
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (q, s) = (queue.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("monitor".into())
            .spawn(move || {
                if let Err(e) = render_loop(rate, &q, &s) {
                    eprintln!("[monitor] {e}");
                }
            })
            .ok();
        Monitor { queue, stop, thread, rate }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Добавляет стерео-сэмплы на воспроизведение. Не больше 0.3 с в очереди,
    /// чтобы прослушивание не отставало от микрофона.
    pub fn push(&self, samples: &[f32]) {
        let mut q = self.queue.lock().unwrap();
        q.extend(samples);
        let max = (self.rate as usize * 3 / 10) * CHANNELS;
        while q.len() > max {
            q.pop_front();
        }
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn render_loop(rate: u32, queue: &Mutex<VecDeque<f32>>, stop: &AtomicBool) -> Result<(), String> {
    wasapi::initialize_mta().ok().map_err(|e| format!("COM: {e}"))?;
    let device = wasapi::DeviceEnumerator::new()
        .and_then(|en| en.get_default_device(&wasapi::Direction::Render))
        .map_err(|e| e.to_string())?;
    let mut client = device.get_iaudioclient().map_err(|e| e.to_string())?;
    let format = wasapi::WaveFormat::new(32, 32, &wasapi::SampleType::Float, rate as usize, CHANNELS, None);
    let mode = wasapi::StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
    client.initialize_client(&format, &wasapi::Direction::Render, &mode).map_err(|e| e.to_string())?;
    let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
    let render = client.get_audiorenderclient().map_err(|e| e.to_string())?;
    client.start_stream().map_err(|e| e.to_string())?;

    let mut bytes: Vec<u8> = Vec::new();
    while !stop.load(Ordering::Acquire) {
        let _ = event.wait_for_event(100);
        let space = client.get_available_space_in_frames().map_err(|e| e.to_string())? as usize;
        if space == 0 {
            continue;
        }
        bytes.clear();
        {
            let mut q = queue.lock().unwrap();
            for _ in 0..space * CHANNELS {
                // Пусто — тишина: устройству всегда нужно что-то отдать.
                bytes.extend_from_slice(&q.pop_front().unwrap_or(0.0).to_le_bytes());
            }
        }
        render.write_to_device(space, &bytes, None).map_err(|e| e.to_string())?;
    }
    let _ = client.stop_stream();
    Ok(())
}
