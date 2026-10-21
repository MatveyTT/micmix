// Приложение MicMix: окно с микшером, иконка в трее, горячая клавиша mute.
// Без консольного окна в релизной сборке.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::sync::Mutex;

use micmix_core::{apps, mics, Config, Engine, EngineStatus, TONE_ID};
use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

struct AppState {
    engine: Engine,
    config: Mutex<Config>,
    tray_mute: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
}

impl AppState {
    fn save(&self) {
        let mut cfg = self.config.lock().unwrap();
        *cfg = self.engine.config(&cfg);
        if let Err(e) = cfg.save() {
            eprintln!("не удалось сохранить настройки: {e}");
        }
    }
}

/// После переключения mute — обновить галочку в трее и сообщить окну.
fn sync_mute(app: &AppHandle, muted: bool) {
    let state = app.state::<AppState>();
    if let Some(item) = state.tray_mute.lock().unwrap().as_ref() {
        let _ = item.set_checked(muted);
    }
    let _ = app.emit("master-muted", muted);
}

#[tauri::command]
async fn get_status(state: State<'_, AppState>) -> Result<EngineStatus, String> {
    Ok(state.engine.status())
}

#[tauri::command]
async fn list_apps() -> Result<Vec<apps::AudioApp>, String> {
    Ok(apps::list_audio_apps())
}

#[tauri::command]
async fn add_source(state: State<'_, AppState>, exe: String, name: String) -> Result<(), String> {
    state.engine.add_source(&exe, &name);
    state.save();
    Ok(())
}

#[tauri::command]
async fn remove_source(state: State<'_, AppState>, exe: String) -> Result<(), String> {
    state.engine.remove_source(&exe);
    state.save();
    Ok(())
}

#[tauri::command]
async fn set_gain(state: State<'_, AppState>, exe: String, gain: f32, save: bool) -> Result<(), String> {
    state.engine.set_gain(&exe, gain);
    if save {
        state.save(); // при перетаскивании ползунка сохраняем только в конце
    }
    Ok(())
}

#[tauri::command]
async fn set_muted(state: State<'_, AppState>, exe: String, muted: bool) -> Result<(), String> {
    state.engine.set_muted(&exe, muted);
    state.save();
    Ok(())
}

#[tauri::command]
async fn set_master_gain(state: State<'_, AppState>, gain: f32, save: bool) -> Result<(), String> {
    state.engine.set_master_gain(gain);
    if save {
        state.save();
    }
    Ok(())
}

#[tauri::command]
async fn set_master_muted(app: AppHandle, state: State<'_, AppState>, muted: bool) -> Result<(), String> {
    state.engine.set_master_muted(muted);
    state.save();
    sync_mute(&app, muted);
    Ok(())
}

/// Тестовый бип в микрофон: проверить, что всё работает, без приложений.
#[tauri::command]
async fn set_test_tone(state: State<'_, AppState>, on: bool) -> Result<(), String> {
    if on {
        state.engine.add_source(TONE_ID, "Тестовый бип");
    } else {
        state.engine.remove_source(TONE_ID);
    }
    Ok(())
}

#[derive(Serialize)]
struct MicsInfo {
    mics: Vec<mics::Mic>,
    system: mics::SystemInfo,
    hotkey: String,
}

#[tauri::command]
async fn list_mics(state: State<'_, AppState>) -> Result<MicsInfo, String> {
    let hotkey = state.config.lock().unwrap().mute_hotkey.clone();
    Ok(MicsInfo { mics: mics::list_mics(), system: mics::system_info(), hotkey })
}

/// Путь к install-apo.ps1: в установленном приложении — в ресурсах, при разработке — в корне репозитория.
fn installer_path(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(dir) = app.path().resource_dir() {
        let p = dir.join("install-apo.ps1");
        if p.exists() {
            return Ok(p);
        }
    }
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("install-apo.ps1");
    if p.exists() {
        return Ok(p);
    }
    Err("не найден install-apo.ps1".into())
}

/// Запускает установщик APO от администратора (Windows покажет окно UAC) и ждёт его.
#[tauri::command]
async fn apo_install(app: AppHandle, endpoint: String, uninstall: bool) -> Result<String, String> {
    if !endpoint.chars().all(|c| c.is_ascii_hexdigit() || "{}-".contains(c)) {
        return Err("неверный идентификатор микрофона".into());
    }
    let script = installer_path(&app)?;
    let script = script.canonicalize().unwrap_or(script);
    let script_str = script.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    let args = format!(
        "-NoProfile -ExecutionPolicy Bypass -File \"{script_str}\" -Quiet -EndpointId {endpoint}{}",
        if uninstall { " -Uninstall" } else { "" }
    );
    let command = format!(
        "try {{ $p = Start-Process powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '{}'; exit $p.ExitCode }} catch {{ exit 1223 }}",
        args.replace('\'', "''")
    );
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let status = tauri::async_runtime::spawn_blocking(move || {
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &command])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    let log = script.parent().map(|d| d.join("logs").join("install-last.log"));
    let tail = log
        .and_then(|p| std::fs::read(p).ok())
        .map(|b| {
            // Журнал Start-Transcript: шапка и подвал отделены строками из звёздочек, вывод — между ними.
            let text = String::from_utf8_lossy(&b).replace('\u{feff}', "");
            let body = text.split("**********************").nth(2).unwrap_or("");
            let lines: Vec<&str> = body.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            lines[lines.len().saturating_sub(4)..].join("\n")
        })
        .unwrap_or_default();
    match status.code() {
        Some(0) => Ok(tail),
        Some(1223) => Err("Установка отменена: нужно разрешить запуск от администратора.".into()),
        _ => Err(if tail.is_empty() { "Установщик завершился с ошибкой.".into() } else { tail }),
    }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn main() {
    let config = Config::load();
    let hotkey = config.mute_hotkey.clone();
    let engine = Engine::start(&config);

    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let muted = app.state::<AppState>().engine.toggle_master_muted();
                        app.state::<AppState>().save();
                        sync_mute(app, muted);
                    }
                })
                .build(),
        )
        .manage(AppState { engine, config: Mutex::new(config), tray_mute: Mutex::new(None) })
        .setup(move |app| {
            if let Err(e) = app.global_shortcut().register(hotkey.as_str()) {
                eprintln!("горячая клавиша {hotkey} не зарегистрирована: {e}");
            }

            let muted = app.state::<AppState>().engine.status().master_muted;
            let open = MenuItem::with_id(app, "open", "Открыть MicMix", true, None::<&str>)?;
            let mute = CheckMenuItem::with_id(app, "mute", "Выключить звук в микрофоне", true, muted, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &mute, &PredefinedMenuItem::separator(app)?, &quit])?;
            *app.state::<AppState>().tray_mute.lock().unwrap() = Some(mute);

            TrayIconBuilder::with_id("tray")
                .icon(app.default_window_icon().cloned().ok_or("нет иконки")?)
                .tooltip("MicMix")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_main(app),
                    "mute" => {
                        let muted = app.state::<AppState>().engine.toggle_master_muted();
                        app.state::<AppState>().save();
                        sync_mute(app, muted);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Крестик прячет окно в трей: звук в микрофон продолжает идти.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            list_apps,
            add_source,
            remove_source,
            set_gain,
            set_muted,
            set_master_gain,
            set_master_muted,
            set_test_tone,
            list_mics,
            apo_install
        ])
        .run(tauri::generate_context!())
        .expect("не удалось запустить MicMix");
}
