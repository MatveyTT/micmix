//! Консольный движок MicMix — для проверки без интерфейса.
//!
//!   micmix-engine tone                         тестовый бип 880 Гц в микрофон
//!   micmix-engine capture <имя.exe> [...]      звук приложений (вместе с дочерними процессами)
//!   micmix-engine apps                         приложения со звуком
//!   micmix-engine mics                         микрофоны и где стоит APO
//!
//! Опции: --gain 0.5 (общая громкость). Настройки приложения не трогает.

use std::time::Duration;

use micmix_core::{apps, mics, Config, Engine, TONE_ID};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut gain = 0.5f32;
    let mut positional = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--gain" => gain = it.next().and_then(|v| v.parse().ok()).unwrap_or(gain),
            _ => positional.push(a.clone()),
        }
    }

    let sources: Vec<String> = match positional.first().map(String::as_str) {
        Some("tone") => vec![TONE_ID.to_string()],
        Some("capture") if positional.len() > 1 => positional[1..].to_vec(),
        Some("apps") => {
            for a in apps::list_audio_apps() {
                println!("{:<28} {:<24} {}", a.exe, a.name, if a.playing { "играет" } else { "" });
            }
            return;
        }
        Some("mics") => {
            for m in mics::list_mics() {
                println!(
                    "{} {} — {}{}",
                    m.endpoint,
                    m.name,
                    m.adapter,
                    if m.apo_installed { "  [APO стоит]" } else { "" }
                );
            }
            return;
        }
        _ => {
            eprintln!("Использование:");
            eprintln!("  micmix-engine tone                      тестовый бип в микрофон");
            eprintln!("  micmix-engine capture <имя.exe> [...]   звук приложений в микрофон");
            eprintln!("  micmix-engine apps | mics               списки приложений и микрофонов");
            eprintln!("Опции: --gain 0.5");
            std::process::exit(2);
        }
    };

    let config = Config { sources: Vec::new(), master_gain: gain, master_muted: false, ..Config::default() };
    let engine = Engine::start(&config);
    for s in &sources {
        engine.add_source(s, s);
    }
    println!("Источники: {}. Ctrl+C — остановить.", sources.join(", "));
    println!("Жду APO: открой запись с микрофона («Звукозапись», Discord)...");
    let mut was_connected = false;
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let st = engine.status();
        if st.apo_connected && !was_connected {
            println!("Подключился к APO ({} Гц) — звук идёт в микрофон.", st.sample_rate);
        }
        was_connected = st.apo_connected;
        for s in st.sources.iter().filter(|s| !s.running) {
            println!("  {} не запущено — жду", s.exe);
        }
    }
}
