//! Горячие клавиши: весь звук в микрофон, все приложения, остановить звуки,
//! отдельно каждое приложение и каждый звук.
//!
//! Все привязки регистрируются заново после любого изменения: так проще
//! поймать повторы и клавиши, занятые другими программами (Discord и т. п.).

use std::collections::HashMap;
use std::str::FromStr;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::AppState;

/// Что делает клавиша.
#[derive(Clone, Debug)]
pub enum Action {
    ToggleMaster,
    ToggleApps,
    StopSounds,
    ToggleSource(String),
    PlaySound(String),
}

/// Ключ привязки — так её называет интерфейс: "master", "apps", "stop",
/// "source:<exe>", "sound:<id>".
pub fn target_key(action: &Action) -> String {
    match action {
        Action::ToggleMaster => "master".into(),
        Action::ToggleApps => "apps".into(),
        Action::StopSounds => "stop".into(),
        Action::ToggleSource(exe) => format!("source:{exe}"),
        Action::PlaySound(id) => format!("sound:{id}"),
    }
}

#[derive(Default)]
pub struct Bindings {
    /// id сочетания (Shortcut::id) → действие.
    by_id: HashMap<u32, Action>,
    /// ключ привязки → почему не работает.
    pub errors: HashMap<String, String>,
}

#[derive(Serialize)]
pub struct HotkeysInfo {
    pub master: String,
    pub apps: Option<String>,
    pub stop: Option<String>,
    pub errors: HashMap<String, String>,
}

/// Все привязки из настроек: (клавиша, действие).
fn collect(app: &AppHandle) -> Vec<(String, Action)> {
    let state = app.state::<AppState>();
    let cfg = state.config.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    let status = state.engine.status();
    let mut list = Vec::new();
    if !cfg.mute_hotkey.is_empty() {
        list.push((cfg.mute_hotkey, Action::ToggleMaster));
    }
    if let Some(k) = cfg.apps_hotkey {
        list.push((k, Action::ToggleApps));
    }
    if let Some(k) = cfg.stop_hotkey {
        list.push((k, Action::StopSounds));
    }
    for s in status.sources {
        if let Some(k) = s.hotkey {
            list.push((k, Action::ToggleSource(s.exe)));
        }
    }
    for s in status.sounds {
        if let Some(k) = s.hotkey {
            list.push((k, Action::PlaySound(s.id)));
        }
    }
    list
}

/// Снимает все клавиши и регистрирует заново. Ошибки — в `Bindings::errors`.
pub fn rebind(app: &AppHandle) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut b = Bindings::default();
    let mut owner: HashMap<u32, String> = HashMap::new(); // id сочетания → кто его занял
    for (key, action) in collect(app) {
        let target = target_key(&action);
        let shortcut = match Shortcut::from_str(&key) {
            Ok(s) => s,
            Err(_) => {
                b.errors.insert(target, format!("{key}: непонятное сочетание"));
                continue;
            }
        };
        if let Some(first) = owner.get(&shortcut.id()) {
            b.errors.insert(target, format!("{key} уже назначена ({})", describe_target(app, first)));
            continue;
        }
        match gs.register(shortcut) {
            Ok(()) => {
                owner.insert(shortcut.id(), target);
                b.by_id.insert(shortcut.id(), action);
            }
            Err(e) => {
                let msg = if e.to_string().contains("already registered") {
                    format!("{key} занята другой программой")
                } else {
                    format!("{key}: {e}")
                };
                b.errors.insert(target, msg);
            }
        }
    }
    for (target, e) in &b.errors {
        eprintln!("[{}] клавиша {target}: {e}", crate::now());
    }
    *app.state::<AppState>().bindings.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = b;
}

/// Человеческое имя привязки для сообщения «уже назначена (...)».
fn describe_target(app: &AppHandle, target: &str) -> String {
    let status = app.state::<AppState>().engine.status();
    match target {
        "master" => "весь звук в микрофон".into(),
        "apps" => "все приложения".into(),
        "stop" => "остановить звуки".into(),
        t => {
            if let Some(exe) = t.strip_prefix("source:") {
                status.sources.iter().find(|s| s.exe == exe).map_or(exe.to_string(), |s| s.name.clone())
            } else if let Some(id) = t.strip_prefix("sound:") {
                status.sounds.iter().find(|s| s.id == id).map_or(id.to_string(), |s| format!("звук «{}»", s.name))
            } else {
                t.to_string()
            }
        }
    }
}

/// Нажата клавиша с этим id.
pub fn on_pressed(app: &AppHandle, id: u32) {
    let action = app.state::<AppState>().bindings.lock().unwrap_or_else(std::sync::PoisonError::into_inner).by_id.get(&id).cloned();
    let Some(action) = action else { return };
    let state = app.state::<AppState>();
    match action {
        Action::ToggleMaster => {
            let muted = state.engine.toggle_master_muted();
            state.save();
            crate::sync_mute(app, muted);
        }
        Action::ToggleApps => {
            state.engine.toggle_apps_muted();
            state.save();
        }
        Action::StopSounds => state.engine.stop_sounds(),
        Action::ToggleSource(exe) => {
            state.engine.toggle_source_muted(&exe);
            state.save();
        }
        Action::PlaySound(id) => {
            // Декодирование может занять время — не в потоке событий клавиатуры.
            let app = app.clone();
            std::thread::spawn(move || {
                if let Err(e) = app.state::<AppState>().engine.play_sound(&id) {
                    let _ = app.emit("sound-error", e);
                }
            });
        }
    }
    let _ = app.emit("state-changed", ());
}
