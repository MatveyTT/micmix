// Приложение MicMix: окно с микшером и звуками, иконка в трее, горячие клавиши.
// Без консольного окна в релизной сборке.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hotkeys;

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use micmix_core::{apps, config, mics, sounds, tab, Config, Engine, EngineStatus, SoundConfig, TAB_ID, TONE_ID};
use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_global_shortcut::ShortcutState;

pub struct AppState {
    engine: Engine,
    config: Mutex<Config>,
    tray_mute: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    bindings: Mutex<hotkeys::Bindings>,
    /// Почему не запустился приём звука вкладки (порт занят и т. п.).
    tab_error: Mutex<Option<String>>,
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

/// Старая клавиша по умолчанию: совпадает с «выключить микрофон» в Discord.
const OLD_DEFAULT_HOTKEY: &str = "Ctrl+Shift+M";

/// Если MicMix уже запущен — показать его окно и вернуть true.
/// Два движка писали бы в одну общую память и портили звук в микрофоне.
fn activate_running_instance() -> bool {
    use windows::core::w;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW};
    unsafe {
        // Мьютекс живёт до конца процесса — специально не закрываем.
        let _mutex = CreateMutexW(None, false, w!("Local\\MicMix.App"));
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return false;
        }
        if let Ok(hwnd) = FindWindowW(None, w!("MicMix")) {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        true
    }
}

/// После переключения mute — обновить галочку в трее и сообщить окну.
pub fn sync_mute(app: &AppHandle, muted: bool) {
    let state = app.state::<AppState>();
    if let Some(item) = state.tray_mute.lock().unwrap().as_ref() {
        let _ = item.set_checked(muted);
    }
    let _ = app.emit("state-changed", ());
}

// ---------- Состояние ----------

#[tauri::command]
async fn get_status(state: State<'_, AppState>) -> Result<EngineStatus, String> {
    Ok(state.engine.status())
}

// ---------- Источники ----------

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
async fn remove_source(app: AppHandle, state: State<'_, AppState>, exe: String) -> Result<(), String> {
    state.engine.remove_source(&exe);
    state.save();
    hotkeys::rebind(&app); // у источника могла быть клавиша
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

// ---------- Общие ----------

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

#[tauri::command]
async fn set_apps_muted(state: State<'_, AppState>, muted: bool) -> Result<(), String> {
    state.engine.set_apps_muted(muted);
    state.save();
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

// ---------- Звуки ----------

fn sounds_dir() -> Result<PathBuf, String> {
    let dir = config::data_dir().ok_or("нет %APPDATA%")?.join("sounds");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Добавляет звуки из файлов. Каждый проверяется декодером и копируется в
/// %APPDATA%\MicMix\sounds. Возвращает ошибки по файлам (пусто — всё добавилось).
#[tauri::command]
async fn add_sounds(state: State<'_, AppState>, paths: Vec<String>) -> Result<Vec<String>, String> {
    let dir = sounds_dir()?;
    let rate = state.engine.sample_rate();
    let mut errors = Vec::new();
    for path in paths {
        let src = Path::new(&path);
        let file_name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.clone());
        let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !sounds::EXTENSIONS.contains(&ext.as_str()) {
            errors.push(format!("{file_name}: такой формат не поддерживается"));
            continue;
        }
        let loaded = tauri::async_runtime::spawn_blocking({
            let src = src.to_path_buf();
            move || sounds::load(&src, rate)
        })
        .await
        .map_err(|e| e.to_string())?;
        let data = match loaded {
            Ok(d) => d,
            Err(e) => {
                errors.push(format!("{file_name}: {e}"));
                continue;
            }
        };
        let id = format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        let copy = dir.join(format!("{id}.{ext}"));
        if let Err(e) = std::fs::copy(src, &copy) {
            errors.push(format!("{file_name}: не удалось скопировать ({e})"));
            continue;
        }
        let name = src.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or(file_name);
        let cfg = SoundConfig { id, name, file: copy.to_string_lossy().to_string(), gain: 1.0, hotkey: None };
        state.engine.add_sound(cfg, data, rate);
    }
    state.save();
    Ok(errors)
}

#[tauri::command]
async fn remove_sound(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    if let Some(file) = state.engine.remove_sound(&id) {
        // Удаляем только свою копию — файлы пользователя не трогаем.
        if let Ok(dir) = sounds_dir() {
            if Path::new(&file).starts_with(&dir) {
                let _ = std::fs::remove_file(&file);
            }
        }
    }
    state.save();
    hotkeys::rebind(&app);
    Ok(())
}

#[tauri::command]
async fn play_sound(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.engine.play_sound(&id)
}

#[tauri::command]
async fn stop_sounds(state: State<'_, AppState>) -> Result<(), String> {
    state.engine.stop_sounds();
    Ok(())
}

#[tauri::command]
async fn set_sound_gain(state: State<'_, AppState>, id: String, gain: f32, save: bool) -> Result<(), String> {
    state.engine.set_sound_gain(&id, gain);
    if save {
        state.save();
    }
    Ok(())
}

#[tauri::command]
async fn rename_sound(state: State<'_, AppState>, id: String, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("пустое название".into());
    }
    state.engine.rename_sound(&id, name);
    state.save();
    Ok(())
}

#[tauri::command]
async fn set_sounds_gain(state: State<'_, AppState>, gain: f32, save: bool) -> Result<(), String> {
    state.engine.set_sounds_gain(gain);
    if save {
        state.save();
    }
    Ok(())
}

#[tauri::command]
async fn set_monitor(state: State<'_, AppState>, on: bool) -> Result<(), String> {
    state.engine.set_monitor(on);
    state.save();
    Ok(())
}

#[tauri::command]
async fn set_monitor_gain(state: State<'_, AppState>, gain: f32, save: bool) -> Result<(), String> {
    state.engine.set_monitor_gain(gain);
    if save {
        state.save();
    }
    Ok(())
}

// ---------- Горячие клавиши ----------

#[tauri::command]
async fn get_hotkeys(state: State<'_, AppState>) -> Result<hotkeys::HotkeysInfo, String> {
    let cfg = state.config.lock().unwrap().clone();
    let errors = state.bindings.lock().unwrap().errors.clone();
    Ok(hotkeys::HotkeysInfo { master: cfg.mute_hotkey, apps: cfg.apps_hotkey, stop: cfg.stop_hotkey, errors })
}

/// Назначает клавишу (`None` — убрать). target: master | apps | stop | source:<exe> | sound:<id>.
/// Если клавиша не заработала (занята, повтор) — возвращается прежняя и ошибка.
#[tauri::command]
async fn set_hotkey(app: AppHandle, state: State<'_, AppState>, target: String, hotkey: Option<String>) -> Result<(), String> {
    let hotkey = hotkey.filter(|h| !h.trim().is_empty());
    let set = |value: Option<String>| -> Option<String> {
        let mut cfg = state.config.lock().unwrap();
        match target.as_str() {
            "master" => Some(std::mem::replace(&mut cfg.mute_hotkey, value.unwrap_or_default())).filter(|s| !s.is_empty()),
            "apps" => std::mem::replace(&mut cfg.apps_hotkey, value),
            "stop" => std::mem::replace(&mut cfg.stop_hotkey, value),
            t => {
                drop(cfg);
                let old = if let Some(exe) = t.strip_prefix("source:") {
                    state.engine.status().sources.into_iter().find(|s| s.exe == exe).and_then(|s| s.hotkey)
                } else if let Some(id) = t.strip_prefix("sound:") {
                    state.engine.status().sounds.into_iter().find(|s| s.id == id).and_then(|s| s.hotkey)
                } else {
                    None
                };
                if let Some(exe) = t.strip_prefix("source:") {
                    state.engine.set_source_hotkey(exe, value);
                } else if let Some(id) = t.strip_prefix("sound:") {
                    state.engine.set_sound_hotkey(id, value);
                }
                old
            }
        }
    };
    let old = set(hotkey);
    hotkeys::rebind(&app);
    let error = state.bindings.lock().unwrap().errors.get(&target).cloned();
    if let Some(e) = error {
        set(old);
        hotkeys::rebind(&app);
        return Err(e);
    }
    state.save();
    Ok(())
}

// ---------- Микрофоны и установка APO ----------

#[derive(Serialize)]
struct MicsInfo {
    mics: Vec<mics::Mic>,
    system: mics::SystemInfo,
    tab_error: Option<String>,
}

#[tauri::command]
async fn list_mics(state: State<'_, AppState>) -> Result<MicsInfo, String> {
    let tab_error = state.tab_error.lock().unwrap().clone();
    Ok(MicsInfo { mics: mics::list_mics(), system: mics::system_info(), tab_error })
}

/// Путь к ресурсу: в установленном приложении — рядом с exe, при разработке — в корне репозитория.
fn resource_path(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    if let Ok(dir) = app.path().resource_dir() {
        let p = dir.join(name);
        if p.exists() {
            return Ok(p);
        }
    }
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(name);
    if p.exists() {
        return Ok(p.canonicalize().unwrap_or(p));
    }
    Err(format!("не найден {name}"))
}

/// Открывает папку с расширением для браузера в Проводнике.
#[tauri::command]
async fn open_extension_folder(app: AppHandle) -> Result<String, String> {
    let dir = resource_path(&app, "extension")?;
    let shown = dir.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    std::process::Command::new("explorer").arg(&shown).spawn().map_err(|e| e.to_string())?;
    Ok(shown)
}

/// Запускает установщик APO от администратора (Windows покажет окно UAC) и ждёт его.
#[tauri::command]
async fn apo_install(app: AppHandle, endpoint: String, uninstall: bool) -> Result<String, String> {
    if !endpoint.chars().all(|c| c.is_ascii_hexdigit() || "{}-".contains(c)) {
        return Err("неверный идентификатор микрофона".into());
    }
    let script = resource_path(&app, "install-apo.ps1")?;
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
    if activate_running_instance() {
        return;
    }
    let config = Config::load();
    let engine = Engine::start(&config);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        hotkeys::on_pressed(app, shortcut.id());
                    }
                })
                .build(),
        )
        .manage(AppState {
            engine,
            config: Mutex::new(config),
            tray_mute: Mutex::new(None),
            bindings: Mutex::new(hotkeys::Bindings::default()),
            tab_error: Mutex::new(None),
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let state = app.state::<AppState>();

            hotkeys::rebind(&handle);
            // Старая клавиша по умолчанию конфликтует с Discord — переезжаем на новую.
            let master_failed = state.bindings.lock().unwrap().errors.contains_key("master");
            if master_failed && state.config.lock().unwrap().mute_hotkey == OLD_DEFAULT_HOTKEY {
                state.config.lock().unwrap().mute_hotkey = Config::default().mute_hotkey;
                hotkeys::rebind(&handle);
                state.save();
            }

            // Приём звука вкладки от расширения браузера.
            if let Err(e) = tab::start(state.engine.feed(TAB_ID)) {
                *state.tab_error.lock().unwrap() = Some(e);
            }

            let muted = state.engine.status().master_muted;
            let open = MenuItem::with_id(app, "open", "Открыть MicMix", true, None::<&str>)?;
            let mute = CheckMenuItem::with_id(app, "mute", "Выключить звук в микрофоне", true, muted, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop", "Остановить звуки", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &mute, &stop, &PredefinedMenuItem::separator(app)?, &quit])?;
            *state.tray_mute.lock().unwrap() = Some(mute);

            TrayIconBuilder::with_id("tray")
                .icon(app.default_window_icon().cloned().ok_or("нет иконки")?)
                .tooltip("MicMix")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    let state = app.state::<AppState>();
                    match event.id().as_ref() {
                        "open" => show_main(app),
                        "mute" => {
                            let muted = state.engine.toggle_master_muted();
                            state.save();
                            sync_mute(app, muted);
                        }
                        "stop" => state.engine.stop_sounds(),
                        "quit" => app.exit(0),
                        _ => {}
                    }
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
            set_apps_muted,
            set_test_tone,
            add_sounds,
            remove_sound,
            play_sound,
            stop_sounds,
            set_sound_gain,
            rename_sound,
            set_sounds_gain,
            set_monitor,
            set_monitor_gain,
            get_hotkeys,
            set_hotkey,
            list_mics,
            open_extension_folder,
            apo_install
        ])
        .run(tauri::generate_context!())
        .expect("не удалось запустить MicMix");
}
