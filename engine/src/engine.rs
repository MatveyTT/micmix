//! Микшер: захватывает звук выбранных приложений, проигрывает звуки-кнопки,
//! сводит всё с громкостями, пропускает через лимитер и пишет в общую память APO.
//!
//! Потоки:
//! - по одному потоку захвата на приложение (WASAPI process loopback) — складывают
//!   звук в свою очередь;
//! - один поток микшера — каждые 5 мс забирает из очередей столько кадров, сколько
//!   прошло времени, сводит и отдаёт APO;
//! - поток прослушивания (только пока играют звуки-кнопки и оно включено).
//!
//! Частоту берём у APO (он знает частоту микрофона), и Windows сама приводит звук
//! приложений к ней при захвате. Звуки-кнопки приводятся к ней при загрузке.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::apps;
use crate::config::{Config, SoundConfig, SourceConfig};
use crate::monitor::Monitor;
use crate::ring::{Ring, CHANNELS};
use crate::sounds;

/// Особый источник: тестовый бип 880 Гц, чтобы проверить микрофон без приложений.
pub const TONE_ID: &str = "micmix:tone";
/// Особый источник: вкладка браузера, звук приходит от расширения MicMix.
pub const TAB_ID: &str = "micmix:tab";

const DEFAULT_RATE: u32 = 48_000;
const PRIME_MS: u32 = 10; // запас в очереди перед стартом источника
const MAX_LAG_MS: u32 = 100; // больше — срезаем до TRIM_MS
const TRIM_MS: u32 = 20;
const LIMIT_CEILING: f32 = 0.89; // −1 дБFS
const MONITOR_IDLE: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Serialize)]
pub struct SourceStatus {
    pub exe: String,
    pub name: String,
    pub icon: String,
    pub gain: f32,
    pub muted: bool,
    pub hotkey: Option<String>,
    /// Пиковый уровень после громкости, 0..1.
    pub level: f32,
    /// Приложение запущено и звук захватывается (для вкладки — расширение подключено).
    pub running: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SoundStatus {
    pub id: String,
    pub name: String,
    pub gain: f32,
    pub hotkey: Option<String>,
    /// 0..1 — сколько проиграно; None — не играет.
    pub progress: Option<f32>,
    pub seconds: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct EngineStatus {
    pub sources: Vec<SourceStatus>,
    pub sounds: Vec<SoundStatus>,
    pub master_gain: f32,
    pub master_muted: bool,
    pub master_level: f32,
    pub apps_muted: bool,
    pub sounds_gain: f32,
    pub sounds_level: f32,
    pub monitor: bool,
    pub monitor_gain: f32,
    /// Есть связь с APO: кто-то пишет с микрофона, на котором стоит APO.
    pub apo_connected: bool,
    pub sample_rate: u32,
    /// Насколько лимитер сейчас приглушает звук, 0..1 (0 — не работает).
    pub limiter: f32,
}

type Queue = Arc<Mutex<VecDeque<f32>>>;

/// Внешний поток звука (вкладка браузера): его пишет сервер расширения.
pub struct Feed {
    pub queue: Queue,
    pub connected: AtomicBool,
    /// Частота микшера — в ней сервер должен класть звук в очередь.
    pub rate: std::sync::atomic::AtomicU32,
}

struct Capture {
    pid: u32,
    queue: Queue,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    started: Instant,
    /// Когда последний раз пришёл звук: миллисекунды от `started`.
    last_data_ms: Arc<std::sync::atomic::AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    fn start(pid: u32, rate: u32) -> Capture {
        let queue: Queue = Arc::new(Mutex::new(VecDeque::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let started = Instant::now();
        let last_data_ms = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (q, s, f, l) = (queue.clone(), stop.clone(), failed.clone(), last_data_ms.clone());
        let thread = std::thread::Builder::new()
            .name(format!("capture-{pid}"))
            .spawn(move || {
                if let Err(e) = capture_loop(pid, rate, &q, &s, started, &l) {
                    eprintln!("[capture {pid}] {e}");
                    f.store(true, Ordering::Release);
                }
            })
            .ok();
        Capture { pid, queue, stop, failed, started, last_data_ms, thread }
    }

    /// Захват идёт уже пару секунд, а звука от процесса нет.
    fn silent(&self) -> bool {
        let age = self.started.elapsed();
        let since_data = age.saturating_sub(Duration::from_millis(self.last_data_ms.load(Ordering::Acquire)));
        age > Duration::from_secs(3) && since_data > Duration::from_secs(2)
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

fn capture_loop(
    pid: u32,
    rate: u32,
    queue: &Mutex<VecDeque<f32>>,
    stop: &AtomicBool,
    started: Instant,
    last_data_ms: &std::sync::atomic::AtomicU64,
) -> Result<(), String> {
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
        last_data_ms.store(started.elapsed().as_millis() as u64, Ordering::Release);
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
    feed: Option<Arc<Feed>>,
    primed: bool,
    level: f32,
    tone_phase: f64,
    tone_frame: u64,
}

impl Source {
    fn new(cfg: SourceConfig) -> Source {
        // Иконку найдёт поток микшера при запуске захвата: здесь нельзя трогать COM —
        // Engine::start вызывается в главном потоке, которому окно Tauri нужно в режиме STA.
        Source { cfg, icon: String::new(), capture: None, feed: None, primed: false, level: 0.0, tone_phase: 0.0, tone_frame: 0 }
    }

    fn is_tone(&self) -> bool {
        self.cfg.exe == TONE_ID
    }

    fn is_app(&self) -> bool {
        !self.cfg.exe.starts_with("micmix:")
    }

    fn running(&self) -> bool {
        self.is_tone() || self.capture.is_some() || self.feed.as_ref().is_some_and(|f| f.connected.load(Ordering::Acquire))
    }

    /// Добавляет в mix `frames` кадров этого источника; возвращает пик.
    fn mix_into(&mut self, mix: &mut [f32], frames: usize, rate: u32, apps_gain: f32) -> f32 {
        let gain = if self.cfg.muted { 0.0 } else { self.cfg.gain * apps_gain };
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
        let queue = match (&self.capture, &self.feed) {
            (Some(c), _) => c.queue.clone(),
            (None, Some(f)) => f.queue.clone(),
            _ => return 0.0,
        };
        let mut q = queue.lock().unwrap();
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

struct Sound {
    cfg: SoundConfig,
    /// Сэмплы в частоте `rate`; None — ещё не загружен.
    data: Option<Arc<Vec<f32>>>,
    rate: u32,
}

struct Voice {
    id: String,
    data: Arc<Vec<f32>>,
    pos: usize, // в сэмплах
}

struct State {
    sources: Vec<Source>,
    sounds: Vec<Sound>,
    voices: Vec<Voice>,
    feeds: HashMap<String, Arc<Feed>>,
    master_gain: f32,
    master_muted: bool,
    master_level: f32,
    apps_muted: bool,
    sounds_gain: f32,
    sounds_level: f32,
    monitor_on: bool,
    monitor_gain: f32,
    monitor: Option<Monitor>,
    last_voice: Instant,
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
            sounds: config.sounds.iter().cloned().map(|cfg| Sound { cfg, data: None, rate: 0 }).collect(),
            voices: Vec::new(),
            feeds: HashMap::new(),
            master_gain: config.master_gain,
            master_muted: config.master_muted,
            master_level: 0.0,
            apps_muted: config.apps_muted,
            sounds_gain: config.sounds_gain,
            sounds_level: 0.0,
            monitor_on: config.monitor,
            monitor_gain: config.monitor_gain,
            monitor: None,
            last_voice: Instant::now(),
            limiter_env: 1.0,
            ring: None,
            rate: DEFAULT_RATE,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp) = (state.clone(), stop.clone());
        let thread = std::thread::Builder::new().name("mixer".into()).spawn(move || mixer_loop(&st, &sp)).ok();
        Engine { state, stop, thread }
    }

    // ---------- Источники ----------

    /// Добавляет приложение по имени exe (или `TONE_ID` / `TAB_ID`). Повторное добавление ничего не делает.
    pub fn add_source(&self, exe: &str, name: &str) {
        let mut s = self.state.lock().unwrap();
        let exe = exe.to_lowercase();
        if s.sources.iter().any(|x| x.cfg.exe == exe) {
            return;
        }
        s.sources.push(Source::new(SourceConfig { exe, name: name.to_string(), gain: 1.0, muted: false, hotkey: None }));
    }

    pub fn remove_source(&self, exe: &str) {
        self.state.lock().unwrap().sources.retain(|x| x.cfg.exe != exe);
    }

    fn with_source<R>(&self, exe: &str, f: impl FnOnce(&mut SourceConfig) -> R) -> Option<R> {
        self.state.lock().unwrap().sources.iter_mut().find(|x| x.cfg.exe == exe).map(|x| f(&mut x.cfg))
    }

    pub fn set_gain(&self, exe: &str, gain: f32) {
        self.with_source(exe, |c| c.gain = gain.clamp(0.0, 2.0));
    }

    pub fn set_muted(&self, exe: &str, muted: bool) {
        self.with_source(exe, |c| c.muted = muted);
    }

    /// Переключает mute источника, возвращает новое значение.
    pub fn toggle_source_muted(&self, exe: &str) -> Option<bool> {
        self.with_source(exe, |c| {
            c.muted = !c.muted;
            c.muted
        })
    }

    pub fn set_source_hotkey(&self, exe: &str, hotkey: Option<String>) {
        self.with_source(exe, |c| c.hotkey = hotkey);
    }

    /// Очередь для внешнего источника (вкладки браузера). Источник сам её подхватит.
    pub fn feed(&self, id: &str) -> Arc<Feed> {
        let mut s = self.state.lock().unwrap();
        let rate = s.rate;
        s.feeds
            .entry(id.to_string())
            .or_insert_with(|| {
                Arc::new(Feed {
                    queue: Arc::new(Mutex::new(VecDeque::new())),
                    connected: AtomicBool::new(false),
                    rate: std::sync::atomic::AtomicU32::new(rate),
                })
            })
            .clone()
    }

    // ---------- Общие ----------

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

    pub fn set_apps_muted(&self, muted: bool) {
        self.state.lock().unwrap().apps_muted = muted;
    }

    /// Переключает «все приложения», возвращает новое значение.
    pub fn toggle_apps_muted(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        s.apps_muted = !s.apps_muted;
        s.apps_muted
    }

    /// Текущая частота микшера (частота микрофона) — для подготовки звуков.
    pub fn sample_rate(&self) -> u32 {
        self.state.lock().unwrap().rate
    }

    // ---------- Звуки ----------

    /// Добавляет звук; `data` — уже загруженные сэмплы в частоте `rate` (проверка файла).
    pub fn add_sound(&self, cfg: SoundConfig, data: Vec<f32>, rate: u32) {
        let mut s = self.state.lock().unwrap();
        s.sounds.retain(|x| x.cfg.id != cfg.id);
        s.sounds.push(Sound { cfg, data: Some(Arc::new(data)), rate });
    }

    /// Убирает звук, возвращает его файл (чтобы удалить копию).
    pub fn remove_sound(&self, id: &str) -> Option<String> {
        let mut s = self.state.lock().unwrap();
        s.voices.retain(|v| v.id != id);
        let pos = s.sounds.iter().position(|x| x.cfg.id == id)?;
        Some(s.sounds.remove(pos).cfg.file)
    }

    fn with_sound<R>(&self, id: &str, f: impl FnOnce(&mut SoundConfig) -> R) -> Option<R> {
        self.state.lock().unwrap().sounds.iter_mut().find(|x| x.cfg.id == id).map(|x| f(&mut x.cfg))
    }

    pub fn set_sound_gain(&self, id: &str, gain: f32) {
        self.with_sound(id, |c| c.gain = gain.clamp(0.0, 2.0));
    }

    pub fn rename_sound(&self, id: &str, name: &str) {
        self.with_sound(id, |c| c.name = name.to_string());
    }

    pub fn set_sound_hotkey(&self, id: &str, hotkey: Option<String>) {
        self.with_sound(id, |c| c.hotkey = hotkey);
    }

    /// Проигрывает звук с начала (если уже играет — перезапускает, как в Soundpad).
    pub fn play_sound(&self, id: &str) -> Result<(), String> {
        let (file, rate, cached) = {
            let s = self.state.lock().unwrap();
            let snd = s.sounds.iter().find(|x| x.cfg.id == id).ok_or("звук не найден")?;
            let cached = snd.data.clone().filter(|_| snd.rate == s.rate);
            (snd.cfg.file.clone(), s.rate, cached)
        };
        // Декодируем без блокировки микшера: длинный файл читается заметное время.
        let data = match cached {
            Some(d) => d,
            None => Arc::new(sounds::load(Path::new(&file), rate)?),
        };
        let mut s = self.state.lock().unwrap();
        if let Some(snd) = s.sounds.iter_mut().find(|x| x.cfg.id == id) {
            snd.data = Some(data.clone());
            snd.rate = rate;
        }
        s.voices.retain(|v| v.id != id);
        s.voices.push(Voice { id: id.to_string(), data, pos: 0 });
        Ok(())
    }

    pub fn stop_sounds(&self) {
        self.state.lock().unwrap().voices.clear();
    }

    pub fn set_sounds_gain(&self, gain: f32) {
        self.state.lock().unwrap().sounds_gain = gain.clamp(0.0, 2.0);
    }

    pub fn set_monitor(&self, on: bool) {
        self.state.lock().unwrap().monitor_on = on;
    }

    pub fn set_monitor_gain(&self, gain: f32) {
        self.state.lock().unwrap().monitor_gain = gain.clamp(0.0, 2.0);
    }

    // ---------- Состояние ----------

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
                    hotkey: x.cfg.hotkey.clone(),
                    level: x.level,
                    running: x.running(),
                })
                .collect(),
            sounds: s
                .sounds
                .iter()
                .map(|x| {
                    let voice = s.voices.iter().find(|v| v.id == x.cfg.id);
                    let seconds = x.data.as_ref().map_or(0.0, |d| d.len() as f32 / CHANNELS as f32 / x.rate.max(1) as f32);
                    SoundStatus {
                        id: x.cfg.id.clone(),
                        name: x.cfg.name.clone(),
                        gain: x.cfg.gain,
                        hotkey: x.cfg.hotkey.clone(),
                        progress: voice.map(|v| v.pos as f32 / v.data.len().max(1) as f32),
                        seconds,
                    }
                })
                .collect(),
            master_gain: s.master_gain,
            master_muted: s.master_muted,
            master_level: s.master_level,
            apps_muted: s.apps_muted,
            sounds_gain: s.sounds_gain,
            sounds_level: s.sounds_level,
            monitor: s.monitor_on,
            monitor_gain: s.monitor_gain,
            apo_connected: s.ring.is_some(),
            sample_rate: s.rate,
            limiter: 1.0 - s.limiter_env,
        }
    }

    /// Текущие настройки (без тестового бипа) — для сохранения. Горячие клавиши
    /// общих действий хранит приложение — они берутся из `base`.
    pub fn config(&self, base: &Config) -> Config {
        let s = self.state.lock().unwrap();
        Config {
            sources: s.sources.iter().filter(|x| !x.is_tone()).map(|x| x.cfg.clone()).collect(),
            sounds: s.sounds.iter().map(|x| x.cfg.clone()).collect(),
            master_gain: s.master_gain,
            master_muted: s.master_muted,
            apps_muted: s.apps_muted,
            sounds_gain: s.sounds_gain,
            monitor: s.monitor_on,
            monitor_gain: s.monitor_gain,
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
    let mut sfx: Vec<f32> = Vec::new();

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
            s.voices.clear(); // звуки были в старой частоте — перезагрузятся при следующем нажатии
            s.monitor = None;
            clock = Instant::now();
            produced = 0;
            last_maintenance = Instant::now() - Duration::from_secs(10);
        }

        // Раз в секунду: запускаем захват для приложений, которые появились, и
        // перезапускаем тех, кто закрылся или перезапустился.
        if last_maintenance.elapsed() > Duration::from_secs(1) {
            last_maintenance = Instant::now();
            for f in s.feeds.values() {
                f.rate.store(s.rate, Ordering::Release);
            }
            for src in s.sources.iter_mut() {
                if src.cfg.exe.starts_with(TAB_ID) {
                    if src.feed.is_none() {
                        src.feed = s.feeds.get(&src.cfg.exe).cloned();
                    }
                    continue;
                }
                if !src.is_app() {
                    continue;
                }
                let dead = src.capture.as_ref().is_some_and(|c| c.failed.load(Ordering::Acquire) || !apps::is_alive(c.pid));
                if dead {
                    src.capture = None;
                }
                // Процессов с одним именем бывает несколько: если звук теперь у другого
                // (вторая копия программы запустилась позже) — переключаемся на него.
                // Windows фиксирует дерево процессов при запуске захвата: если приложение
                // играет, а звука нет (аудиопроцесс появился позже) — перезапускаем захват.
                let found = apps::find_capture_target(&src.cfg.exe);
                let target = found.map(|(pid, _)| pid);
                let playing = found.is_some_and(|(_, p)| p);
                if src.capture.as_ref().is_some_and(|c| target.is_some_and(|t| t != c.pid) || (playing && c.silent())) {
                    src.capture = None;
                }
                if src.capture.is_none() {
                    if let Some(pid) = target {
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
        let apps_gain = if s.apps_muted { 0.0 } else { 1.0 };
        for src in &mut s.sources {
            let peak = src.mix_into(&mut mix, frames, rate, apps_gain);
            src.level = peak.max(src.level * decay);
        }

        // Звуки-кнопки: отдельно, чтобы их копию отдать в наушники.
        sfx.clear();
        sfx.resize(frames * CHANNELS, 0.0);
        let sounds_gain = s.sounds_gain;
        let mut sfx_peak = 0.0f32;
        for v in &mut s.voices {
            let gain = s.sounds.iter().find(|x| x.cfg.id == v.id).map_or(0.0, |x| x.cfg.gain) * sounds_gain;
            let n = (v.data.len() - v.pos).min(sfx.len());
            for (dst, src) in sfx[..n].iter_mut().zip(&v.data[v.pos..v.pos + n]) {
                *dst += src * gain;
            }
            v.pos += n;
        }
        s.voices.retain(|v| v.pos < v.data.len());
        for (m, x) in mix.iter_mut().zip(&sfx) {
            *m += x;
            sfx_peak = sfx_peak.max(x.abs());
        }
        s.sounds_level = sfx_peak.max(s.sounds_level * decay);

        // Прослушивание: поток вывода живёт, пока играют звуки (+3 с), и только если включено.
        if !s.voices.is_empty() || sfx_peak > 0.0 {
            s.last_voice = Instant::now();
        }
        let want_monitor = s.monitor_on && s.last_voice.elapsed() < MONITOR_IDLE;
        if want_monitor && s.monitor.as_ref().map_or(true, |m| m.rate() != rate) {
            s.monitor = Some(Monitor::start(rate));
        } else if !want_monitor && s.monitor.is_some() {
            s.monitor = None;
        }
        if let Some(m) = &s.monitor {
            let g = s.monitor_gain;
            let out: Vec<f32> = sfx.iter().map(|x| (x * g).clamp(-1.0, 1.0)).collect();
            m.push(&out);
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
