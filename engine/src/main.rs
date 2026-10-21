//! Движок MicMix (этап 1): берёт звук приложения и отдаёт его APO через общую память.
//!
//!   micmix-engine tone                       тестовый бип 880 Гц в микрофон
//!   micmix-engine capture <PID | имя.exe>    звук приложения (вместе с дочерними процессами)
//!
//! Опции: --gain 0.5 (громкость, по умолчанию 0.5), --rate 48000 (частота микрофона).

mod ring;

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ring::{Ring, CHANNELS};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

struct Options {
    gain: f32,
    rate: u32,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut opts = Options { gain: 0.5, rate: 48_000 };
    let mut positional = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--gain" => opts.gain = it.next().and_then(|v| v.parse().ok()).unwrap_or(opts.gain),
            "--rate" => opts.rate = it.next().and_then(|v| v.parse().ok()).unwrap_or(opts.rate),
            _ => positional.push(a.clone()),
        }
    }

    let result = match positional.first().map(String::as_str) {
        Some("tone") => run_tone(&opts),
        Some("capture") => match positional.get(1) {
            Some(target) => run_capture(target, &opts),
            None => Err("укажи PID или имя процесса: micmix-engine capture chrome.exe".into()),
        },
        _ => {
            eprintln!("Использование:");
            eprintln!("  micmix-engine tone                     тестовый бип в микрофон");
            eprintln!("  micmix-engine capture <PID | имя.exe>  звук приложения в микрофон");
            eprintln!("Опции: --gain 0.5  --rate 48000");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("[!] {e}");
        std::process::exit(1);
    }
}

/// Держит соединение с APO: если секции ещё нет, раз в секунду пробует снова.
struct Output {
    ring: Option<Ring>,
    rate: u32,
    gain: f32,
    last_try: Option<Instant>,
}

impl Output {
    fn new(opts: &Options) -> Self {
        Output { ring: None, rate: opts.rate, gain: opts.gain, last_try: None }
    }

    fn write(&mut self, interleaved: &mut [f32]) {
        if self.ring.is_none() && self.last_try.map_or(true, |t| t.elapsed() > Duration::from_secs(1)) {
            self.last_try = Some(Instant::now());
            self.ring = Ring::open(self.rate);
            if self.ring.is_some() {
                println!("Подключился к APO — звук идёт в микрофон.");
            }
        }
        if let Some(ring) = &mut self.ring {
            for s in interleaved.iter_mut() {
                *s *= self.gain;
            }
            ring.write(interleaved);
        }
    }
}

fn run_tone(opts: &Options) -> Result<(), String> {
    println!("Тестовый бип 880 Гц (0.3 с раз в секунду). Ctrl+C — остановить.");
    println!("Жду APO: открой запись с микрофона («Звукозапись», Discord)...");
    let mut out = Output::new(opts);
    let rate = opts.rate as f64;
    let step = 2.0 * std::f64::consts::PI * 880.0 / rate;
    let start = Instant::now();
    let mut produced: u64 = 0;
    let mut phase = 0.0f64;
    let mut buf = Vec::new();
    loop {
        std::thread::sleep(Duration::from_millis(5));
        // Сколько кадров должно было прозвучать к этому моменту — так не копится дрейф.
        let due = (start.elapsed().as_secs_f64() * rate) as u64;
        buf.clear();
        while produced < due {
            let in_second = produced % opts.rate as u64;
            let s = if in_second < opts.rate as u64 * 3 / 10 {
                phase = (phase + step) % (2.0 * std::f64::consts::PI);
                (phase.sin() * 0.16) as f32
            } else {
                0.0
            };
            buf.extend_from_slice(&[s; CHANNELS]);
            produced += 1;
        }
        out.write(&mut buf);
    }
}

fn run_capture(target: &str, opts: &Options) -> Result<(), String> {
    let pid = match target.parse::<u32>() {
        Ok(pid) => pid,
        Err(_) => find_root_process(target)?,
    };
    println!("Захватываю звук процесса {pid} и его дочерних процессов. Ctrl+C — остановить.");

    wasapi::initialize_mta().ok().map_err(|e| format!("COM: {e}"))?;
    let mut client = wasapi::AudioClient::new_application_loopback_client(pid, true)
        .map_err(|e| format!("не удалось создать захват процесса: {e}"))?;
    let format = wasapi::WaveFormat::new(32, 32, &wasapi::SampleType::Float, opts.rate as usize, CHANNELS, None);
    let mode = wasapi::StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
    client
        .initialize_client(&format, &wasapi::Direction::Capture, &mode)
        .map_err(|e| format!("initialize: {e}"))?;
    let event = client.set_get_eventhandle().map_err(|e| format!("event: {e}"))?;
    let capture = client.get_audiocaptureclient().map_err(|e| format!("capture client: {e}"))?;

    let mut out = Output::new(opts);
    let mut bytes: VecDeque<u8> = VecDeque::new();
    let mut samples: Vec<f32> = Vec::new();
    client.start_stream().map_err(|e| format!("start: {e}"))?;
    println!("Жду APO: открой запись с микрофона («Звукозапись», Discord)...");
    loop {
        // Приложение молчит — событий нет; ждём и пробуем снова.
        let _ = event.wait_for_event(200);
        capture
            .read_from_device_to_deque(&mut bytes)
            .map_err(|e| format!("чтение: {e}"))?;
        let n = bytes.len() / 4;
        samples.clear();
        samples.extend((0..n).map(|_| {
            let b = [bytes.pop_front().unwrap(), bytes.pop_front().unwrap(),
                     bytes.pop_front().unwrap(), bytes.pop_front().unwrap()];
            f32::from_le_bytes(b)
        }));
        if !samples.is_empty() {
            out.write(&mut samples);
        }
    }
}

/// Находит PID «корневого» процесса по имени: у Chrome и Edge много процессов,
/// звук всех вкладок идёт через дочерние, поэтому берём того, чей родитель — не он же.
fn find_root_process(name: &str) -> Result<u32, String> {
    let want = name.to_lowercase();
    let mut procs: Vec<(u32, u32, String)> = Vec::new(); // (pid, parent, имя)
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).map_err(|e| e.to_string())?;
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            let exe = String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase();
            procs.push((e.th32ProcessID, e.th32ParentProcessID, exe));
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    let matches: Vec<_> = procs.iter().filter(|p| p.2 == want || p.2 == format!("{want}.exe")).collect();
    let root = matches
        .iter()
        .find(|p| !matches.iter().any(|q| q.0 == p.1))
        .or(matches.first())
        .ok_or_else(|| format!("процесс «{name}» не запущен"))?;
    Ok(root.0)
}
