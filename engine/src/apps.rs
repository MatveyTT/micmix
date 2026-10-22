//! Приложения со звуком, процессы и их иконки.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

/// Приложение, у которого есть звуковая сессия (оно играет или недавно играло звук).
#[derive(Clone, Debug, Serialize)]
pub struct AudioApp {
    /// Имя exe в нижнем регистре — ключ источника («chrome.exe»).
    pub exe: String,
    /// Человеческое название («Google Chrome»).
    pub name: String,
    /// Иконка как data:image/png;base64,...; пустая строка, если не нашлась.
    pub icon: String,
    /// Звучит прямо сейчас.
    pub playing: bool,
}

/// Список приложений со звуковыми сессиями на всех устройствах вывода.
pub fn list_audio_apps() -> Vec<AudioApp> {
    let mut by_exe: HashMap<String, AudioApp> = HashMap::new();
    for (pid, playing) in session_pids() {
        if pid == std::process::id() {
            continue; // мы сами
        }
        let Some(path) = process_path(pid) else { continue };
        let exe = file_name(&path);
        by_exe
            .entry(exe.clone())
            .and_modify(|a| a.playing |= playing)
            .or_insert_with(|| {
                let (name, icon) = describe(&path);
                AudioApp { exe, name, icon, playing }
            });
    }
    let mut apps: Vec<AudioApp> = by_exe.into_values().collect();
    apps.sort_by(|a, b| b.playing.cmp(&a.playing).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    apps
}

/// Название и иконка exe (с кэшем: иконку извлекать дорого).
pub fn describe(path: &str) -> (String, String) {
    static CACHE: OnceLock<Mutex<HashMap<String, (String, String)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().unwrap().get(path) {
        return v.clone();
    }
    let name = file_description(path).unwrap_or_else(|| {
        let f = file_name(path);
        f.strip_suffix(".exe").unwrap_or(&f).to_string()
    });
    let icon = icon_png(path).map(|png| format!("data:image/png;base64,{}", base64(&png))).unwrap_or_default();
    let v = (name, icon);
    cache.lock().unwrap().insert(path.to_string(), v.clone());
    v
}

/// Полный путь к exe процесса.
pub fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// Жив ли процесс.
pub fn is_alive(pid: u32) -> bool {
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code).is_ok();
        let _ = CloseHandle(h);
        ok && code == STILL_ACTIVE
    }
}

pub fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_lowercase()
}

/// Все процессы: (pid, родитель, имя exe в нижнем регистре).
fn snapshot() -> Vec<(u32, u32, String)> {
    let mut procs = Vec::new();
    unsafe {
        let Ok(snap): Result<HANDLE, _> = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return procs };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            procs.push((e.th32ProcessID, e.th32ParentProcessID, String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase()));
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    procs
}

/// PID звуковых сессий на устройствах вывода: (pid, играет ли сейчас).
fn session_pids() -> Vec<(u32, bool)> {
    let _ = wasapi::initialize_mta();
    let mut out = Vec::new();
    let Ok(enumerator) = wasapi::DeviceEnumerator::new() else { return out };
    let Ok(devices) = enumerator.get_device_collection(&wasapi::Direction::Render) else { return out };
    for device in &devices {
        let Ok(device) = device else { continue };
        let Ok(manager) = device.get_iaudiosessionmanager() else { continue };
        let Ok(sessions) = manager.get_audiosessionenumerator() else { continue };
        for i in 0..sessions.get_count().unwrap_or(0) {
            let Ok(control) = sessions.get_session(i) else { continue };
            let pid = control.get_process_id().unwrap_or(0);
            let playing = control.get_state().map(|s| s == wasapi::SessionState::Active).unwrap_or(false);
            if pid != 0 {
                out.push((pid, playing));
            }
        }
    }
    out
}

/// Какой процесс захватывать для приложения `exe` (захват идёт вместе с дочерними процессами).
///
/// Процессов с одним именем бывает много (Chrome, Edge, PowerShell). Берём тот, у которого
/// есть звуковая сессия (лучше — играющая), и поднимаемся ровно на один уровень, если родитель
/// с тем же именем (у Chrome звук играет аудиопроцесс — прямой ребёнок главного). Выше не идём:
/// Windows захватывает только прямых детей, внуки в захват не попадают (проверено).
/// Сессий нет — берём корневой процесс.
pub fn find_root_process(exe: &str) -> Option<u32> {
    find_capture_target(exe).map(|(pid, _)| pid)
}

/// То же, что `find_root_process`, плюс играет ли сейчас звук у этого приложения.
pub fn find_capture_target(exe: &str) -> Option<(u32, bool)> {
    let want = exe.to_lowercase();
    let want = if want.ends_with(".exe") { want } else { format!("{want}.exe") };
    let procs = snapshot();
    let named = |pid: u32| procs.iter().find(|p| p.0 == pid && p.2 == want);
    let parent_or_self = |pid: u32| named(pid).map(|p| p.1).filter(|&parent| named(parent).is_some()).unwrap_or(pid);

    let mut sessions: Vec<(u32, bool)> = session_pids().into_iter().filter(|(pid, _)| named(*pid).is_some()).collect();
    sessions.sort_by_key(|(_, playing)| !playing);
    if let Some((pid, playing)) = sessions.first() {
        return Some((parent_or_self(*pid), *playing));
    }
    let mine: Vec<&(u32, u32, String)> = procs.iter().filter(|p| p.2 == want).collect();
    mine.iter().find(|p| named(p.1).is_none()).or(mine.first()).map(|p| (p.0, false))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// FileDescription из ресурсов версии exe («Google Chrome», «Spotify»).
fn file_description(path: &str) -> Option<String> {
    unsafe {
        let wpath = wide(path);
        let size = GetFileVersionInfoSizeW(PCWSTR(wpath.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(wpath.as_ptr()), None, size, data.as_mut_ptr() as *mut c_void).ok()?;

        let mut ptr: *mut c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let key = wide("\\VarFileInfo\\Translation");
        if !VerQueryValueW(data.as_ptr() as *const c_void, PCWSTR(key.as_ptr()), &mut ptr, &mut len).as_bool() || len < 4 {
            return None;
        }
        let lang = *(ptr as *const u16);
        let codepage = *(ptr as *const u16).add(1);
        let key = wide(&format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\FileDescription"));
        if !VerQueryValueW(data.as_ptr() as *const c_void, PCWSTR(key.as_ptr()), &mut ptr, &mut len).as_bool() || len < 2 {
            return None;
        }
        let text = std::slice::from_raw_parts(ptr as *const u16, len as usize);
        let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        let s = String::from_utf16_lossy(&text[..end]).trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

/// Большая иконка exe в PNG (32-битная BGRA → RGBA).
fn icon_png(path: &str) -> Option<Vec<u8>> {
    unsafe {
        let wpath = wide(path);
        let mut icon = HICON::default();
        if ExtractIconExW(PCWSTR(wpath.as_ptr()), 0, Some(&mut icon), None, 1) == 0 || icon.is_invalid() {
            return None;
        }
        let result = hicon_to_png(icon);
        let _ = DestroyIcon(icon);
        result
    }
}

unsafe fn hicon_to_png(icon: HICON) -> Option<Vec<u8>> {
    let mut info = ICONINFO::default();
    GetIconInfo(icon, &mut info).ok()?;
    let color = info.hbmColor;
    let cleanup = || {
        let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
    };
    if color.is_invalid() {
        cleanup();
        return None; // монохромная иконка — редкость, пропускаем
    }
    let mut bm = BITMAP::default();
    if GetObjectW(HGDIOBJ(color.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void)) == 0 {
        cleanup();
        return None;
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight);
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        biHeight: -h, // сверху вниз
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let dc = GetDC(None);
    let lines = GetDIBits(dc, color, 0, h as u32, Some(pixels.as_mut_ptr() as *mut c_void), &mut bmi, DIB_RGB_COLORS);
    ReleaseDC(None, dc);
    cleanup();
    if lines == 0 {
        return None;
    }
    // BGRA → RGBA. Если альфа-канала нет совсем (старые иконки), делаем пиксели непрозрачными.
    let has_alpha = pixels.chunks(4).any(|p| p[3] != 0);
    for p in pixels.chunks_mut(4) {
        p.swap(0, 2);
        if !has_alpha {
            p[3] = 255;
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(&pixels).ok()?;
    }
    Some(out)
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}
