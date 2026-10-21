//! Микшер: захватывает звук выбранных приложений, сводит их с громкостью
//! каждого, пропускает через лимитер и пишет в общую память APO.
//!
//! Потоки:
//! - по одному потоку захвата на приложение (WASAPI process loopback) — складывают
//!   звук в свою очередь;
//! - один поток микшера — каждые 5 мс забирает из очередей столько кадров, сколько
//!   прошло времени, сводит и отдаёт APO.
//!
//! Частоту берём у APO (он знает частоту микрофона), и Windows сама приводит звук
//! приложений к ней при захвате, поэтому ресемплер не нужен.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::apps;
use crate::config::{Config, SourceConfig};
use crate::ring::{Ring, CHANNELS};

/// Особый источник: тестовый бип 880 Гц, чтобы проверить микрофон без приложений.
pub const TONE_ID: &str = "micmix:tone";

const DEFAULT_RATE: u32 = 48_000;
const PRIME_MS: u32 = 10; // запас в очереди перед стартом источника
const MAX_LAG_MS: u32 = 100; // больше — срезаем до TRIM_MS
const TRIM_MS: u32 = 20;
const LIMIT_CEILING: f32 = 0.89; // −1 дБFS

#[derive(Clone, Debug, Serialize)]
pub struct SourceStatus {
    pub exe: String,
    pub name: String,
    pub icon: String,
    pub gain: f32,
    pub muted: bool,
    /// Пиковый уровень после громкости, 0..1.
    pub level: f32,
    /// Приложение запущено и звук захватывается.
    pub running: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct EngineStatus {
    pub sources: Vec<SourceStatus>,
    pub master_gain: f32,
    pub master_muted: bool,
    pub master_level: f32,
    /// Есть связь с APO: кто-то пишет с микрофона, на котором стоит APO.
    pub apo_connected: bool,
    pub sample_rate: u32,
    /// Насколько лимитер сейчас приглушает звук, 0..1 (0 — не работает).
    pub limiter: f32,
}

struct Capture {
    pid: u32,
    queue: Arc<Mutex<VecDeque<f32>>>,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    fn start(pid: u32, rate: u32) -> Capture {
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let (q, s, f) = (queue.clone(), stop.clone(), failed.clone());
        let thread = std::thread::Builder::new()
            .name(format!("capture-{pid}"))
            .spawn(move || {
                if let Err(e) = capture_loop(pid, rate, &q, &s) {
                    eprintln!("[capture {pid}] {e}");
                    f.store(true, Ordering::Release);
                }
            })
            .ok();
        Capture { pid, queue, stop, failed, thread }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn capture_loop(pid: u32, rate: u32, queue: &Mutex<VecDeque<f32>>, stop: &AtomicBool) -> Result<(), String> {
    wasapi::initialize_mta().ok().map_err(|e| format!("COM: {e}"))?;
    let mut client = wasapi::AudioClient::new_application_loopback_client(pid, true).map_err(|e| e.to_string())?;
    let format = wasapi::WaveFormat::new(32, 32, &wasapi::SampleType::Float, rate as usize, CHANNELS, None);
    let mode = wasapi::StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
    client.initialize_client(&format, &wasapi::Direction::Capture, &mode).map_err(|e| e.to_string())?;
    let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
    let capture = client.get_audiocaptureclient().map_err(|e| e.to_string())?;
    client.start_stream().map_err(|e| e.to_string())?;

    let max_samples = (rate as usize / 2) * CHANNELS; // не больше 0.5 с в очереди
    let mut bytes: VecDeque<u8> = VecDeque::new();
    while !stop.load(Ordering::Acquire) {
        let _ = event.wait_for_event(100); // приложение молчит — событий нет, это нормально
        if capture.get_next_packet_size().map_err(|e| e.to_string())?.unwrap_or(0) == 0 {
            continue;
        }
        capture.read_from_device_to_deque(&mut bytes).map_err(|e| e.to_string())?;
        let mut q = queue.lock().unwrap();
        while bytes.len() >= 4 {
            let b = [bytes.pop_front().unwrap(), bytes.pop_front().unwrap(), bytes.pop_front().unwrap(), bytes.pop_front().unwrap()];
            q.push_back(f32::from_le_bytes(b));
        }
        while q.len() > max_samples {
            q.pop_front();
        }
    }
    let _ = client.stop_stream();
    Ok(())
}

struct Source {
    cfg: SourceConfig,
    icon: String,
    capture: Option<Capture>,
    primed: bool,
    level: f32,
    tone_phase: f64,
    tone_frame: u64,
}

impl Source {
    fn new(cfg: SourceConfig) -> Source {
        // Иконку найдёт поток микшера при запуске захвата: здесь нельзя трогать COM —
        // Engine::start вызывается в главном потоке, которому окно Tauri нужно в режиме STA.
        Source { cfg, icon: String::new(), capture: None, primed: false, level: 0.0, tone_phase: 0.0, tone_frame: 0 }
    }

    fn is_tone(&self) -> bool {
        self.cfg.exe == TONE_ID
    }

    /// Добавляет в mix `frames` кадров этого источника; возвращает пик.
    fn mix_into(&mut self, mix: &mut [f32], frames: usize, rate: u32) -> f32 {
        let gain = if self.cfg.muted { 0.0 } else { self.cfg.gain };
        let mut peak = 0.0f32;
        if self.is_tone() {
            let step = 2.0 * std::f64::consts::PI * 880.0 / rate as f64;
            for i in 0..frames {
                let on = self.tone_frame % (rate as u64) < (rate as u64) * 3 / 10;
                self.tone_frame += 1;
                let s = if on {
                    self.tone_phase = (self.tone_phase + step) % (2.0 * std::f64::consts::PI);
                    self.tone_phase.sin() as f32 * 0.25 * gain
                } else {
                    0.0
                };
                mix[i * 2] += s;
                mix[i * 2 + 1] += s;
                peak = peak.max(s.abs());
            }
            return peak;
        }
        let Some(cap) = &self.capture else { return 0.0 };
        let mut q = cap.queue.lock().unwrap();
        let mut avail = q.len() / CHANNELS;
        let ms = |m: u32| (rate * m / 1000) as usize;
        if !self.primed {
            if avail < ms(PRIME_MS) {
                return 0.0;
            }
            self.primed = true;
        }
        if avail > ms(MAX_LAG_MS) {
            // Очередь разрослась (часы приложения и микрофона расходятся) — срезаем задержку.
            q.drain(..(avail - ms(TRIM_MS)) * CHANNELS);
            avail = ms(TRIM_MS);
        }
        let take = avail.min(frames);
        for (i, s) in q.drain(..take * CHANNELS).enumerate() {
            let v = s * gain;
            mix[i] += v;
            peak = peak.max(v.abs());
        }
        if take < frames {
            self.primed = false; // приложение замолчало или не успело — ждём новый запас
        }
        peak
    }
}

struct State {
    sources: Vec<Source>,
    master_gain: f32,
    master_muted: bool,
    master_level: f32,
    limiter_env: f32,
    ring: Option<Ring>,
    rate: u32,
}

pub struct Engine {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn start(config: &Config) -> Engine {
        let state = Arc::new(Mutex::new(State {
            sources: config.sources.iter().cloned().map(Source::new).collect(),
            master_gain: config.master_gain,
            master_muted: config.master_muted,
            master_level: 0.0,
            limiter_env: 1.0,
            ring: None,
            rate: DEFAULT_RATE,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp) = (state.clone(), stop.clone());
        let thread = std::thread::Builder::new().name("mixer".into()).spawn(move || mixer_loop(&st, &sp)).ok();
        Engine { state, stop, thread }
    }

    /// Добавляет приложение по имени exe (или `TONE_ID`). Повторное добавление ничего не делает.
    pub fn add_source(&self, exe: &str, name: &str) {
        let mut s = self.state.lock().unwrap();
        let exe = exe.to_lowercase();
        if s.sources.iter().any(|x| x.cfg.exe == exe) {
            return;
        }
        s.sources.push(Source::new(SourceConfig { exe, name: name.to_string(), gain: 1.0, muted: false }));
    }

    pub fn remove_source(&self, exe: &str) {
        self.state.lock().unwrap().sources.retain(|x| x.cfg.exe != exe);
    }

    pub fn set_gain(&self, exe: &str, gain: f32) {
        if let Some(x) = self.state.lock().unwrap().sources.iter_mut().find(|x| x.cfg.exe == exe) {
            x.cfg.gain = gain.clamp(0.0, 2.0);
        }
    }

    pub fn set_muted(&self, exe: &str, muted: bool) {
        if let Some(x) = self.state.lock().unwrap().sources.iter_mut().find(|x| x.cfg.exe == exe) {
            x.cfg.muted = muted;
        }
    }

    pub fn set_master_gain(&self, gain: f32) {
        self.state.lock().unwrap().master_gain = gain.clamp(0.0, 2.0);
    }

    pub fn set_master_muted(&self, muted: bool) {
        self.state.lock().unwrap().master_muted = muted;
    }

    /// Переключает общий mute, возвращает новое значение.
    pub fn toggle_master_muted(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        s.master_muted = !s.master_muted;
        s.master_muted
    }

    pub fn status(&self) -> EngineStatus {
        let s = self.state.lock().unwrap();
        EngineStatus {
            sources: s
                .sources
                .iter()
                .map(|x| SourceStatus {
                    exe: x.cfg.exe.clone(),
                    name: x.cfg.name.clone(),
                    icon: x.icon.clone(),
                    gain: x.cfg.gain,
                    muted: x.cfg.muted,
                    level: x.level,
                    running: x.is_tone() || x.capture.is_some(),
                })
                .collect(),
            master_gain: s.master_gain,
            master_muted: s.master_muted,
            master_level: s.master_level,
            apo_connected: s.ring.is_some(),
            sample_rate: s.rate,
            limiter: 1.0 - s.limiter_env,
        }
    }

    /// Текущие настройки (без тестового бипа) — для сохранения.
    pub fn config(&self, base: &Config) -> Config {
        let s = self.state.lock().unwrap();
        Config {
            sources: s.sources.iter().filter(|x| !x.is_tone()).map(|x| x.cfg.clone()).collect(),
            master_gain: s.master_gain,
            master_muted: s.master_muted,
            ..base.clone()
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn mixer_loop(state: &Mutex<State>, stop: &AtomicBool) {
    let mut clock = Instant::now();
    let mut produced: u64 = 0;
    let mut last_ring_try: Option<Instant> = None;
    let mut last_maintenance = Instant::now() - Duration::from_secs(10);
    let mut mix: Vec<f32> = Vec::new();

    while !stop.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(5));
        let mut s = state.lock().unwrap();
        let s = &mut *s;

        // Связь с APO: секция появляется, когда кто-то пишет с микрофона.
        if s.ring.is_none() && last_ring_try.map_or(true, |t| t.elapsed() > Duration::from_secs(1)) {
            last_ring_try = Some(Instant::now());
            s.ring = Ring::open();
            if let Some(r) = &s.ring {
                r.set_rate(s.rate);
            }
        }
        // Частота микрофона изменилась — перезапускаем захват в новой частоте.
        let mic_rate = s.ring.as_ref().map(|r| r.mic_rate()).filter(|&r| r > 0).unwrap_or(s.rate);
        if mic_rate != s.rate {
            s.rate = mic_rate;
            if let Some(r) = &s.ring {
                r.set_rate(mic_rate);
            }
            for src in &mut s.sources {
                src.capture = None;
                src.primed = false;
            }
            clock = Instant::now();
            produced = 0;
            last_maintenance = Instant::now() - Duration::from_secs(10);
        }

        // Раз в секунду: запускаем захват для приложений, которые появились, и
        // перезапускаем тех, кто закрылся или перезапустился.
        if last_maintenance.elapsed() > Duration::from_secs(1) {
            last_maintenance = Instant::now();
            for src in s.sources.iter_mut().filter(|x| !x.is_tone()) {
                let dead = src.capture.as_ref().is_some_and(|c| c.failed.load(Ordering::Acquire) || !apps::is_alive(c.pid));
                if dead {
                    src.capture = None;
                }
                if src.capture.is_none() {
                    if let Some(pid) = apps::find_root_process(&src.cfg.exe) {
                        if src.icon.is_empty() {
                            if let Some(p) = apps::process_path(pid) {
                                src.icon = apps::describe(&p).1;
                            }
                        }
                        src.capture = Some(Capture::start(pid, s.rate));
                        src.primed = false;
                    }
                }
            }
        }

        // Сколько кадров должно было прозвучать к этому моменту (без накопления дрейфа).
        let rate = s.rate;
        let due = (clock.elapsed().as_secs_f64() * rate as f64) as u64;
        if due.saturating_sub(produced) > rate as u64 / 5 {
            produced = due - rate as u64 / 100; // долго спали (сон ноутбука и т. п.) — не догоняем
        }
        let frames = due.saturating_sub(produced) as usize;
        if frames == 0 {
            continue;
        }
        produced += frames as u64;

        mix.clear();
        mix.resize(frames * CHANNELS, 0.0);
        let decay = (-(frames as f32) / (rate as f32 * 0.3)).exp(); // спад индикатора ~0.3 с
        for src in &mut s.sources {
            let peak = src.mix_into(&mut mix, frames, rate);
            src.level = peak.max(src.level * decay);
        }

        // Общая громкость + лимитер: мгновенная атака, отпускание ~100 мс.
        let master = if s.master_muted { 0.0 } else { s.master_gain };
        let release = 1.0 - (-1.0 / (rate as f32 * 0.1)).exp();
        let mut env = s.limiter_env;
        let mut peak = 0.0f32;
        for f in mix.chunks_mut(CHANNELS) {
            let p = f[0].abs().max(f[1].abs()) * master;
            let need = if p > LIMIT_CEILING { LIMIT_CEILING / p } else { 1.0 };
            env = if need < env { need } else { env + (1.0 - env) * release };
            let g = master * env;
            f[0] *= g;
            f[1] *= g;
            peak = peak.max(f[0].abs()).max(f[1].abs());
        }
        s.limiter_env = env;
        s.master_level = peak.max(s.master_level * decay);

        if let Some(r) = &mut s.ring {
            r.write(&mix);
        }
    }
}
