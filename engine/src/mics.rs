//! Микрофоны и состояние установки APO (чтение реестра, права администратора не нужны).

use serde::Serialize;
use windows::core::PCWSTR;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_MULTI_SZ, RRF_RT_REG_SZ};

/// CLSID нашего APO — тот же, что в MicMixApo.cpp и install-apo.ps1.
pub const APO_CLSID: &str = "{04206D4E-6467-414B-B052-6AA2220324A5}";
const CAPTURE_ROOT: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture";
const PK_SFX: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},5";
const PK_COMPOSITE_SFX: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},13";

#[derive(Clone, Debug, Serialize)]
pub struct Mic {
    /// GUID конечной точки — так его понимает install-apo.ps1 (-EndpointId).
    pub endpoint: String,
    pub name: String,
    pub adapter: String,
    pub is_default: bool,
    pub apo_installed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SystemInfo {
    /// DisableProtectedAudioDG = 1: Windows загружает неподписанные APO.
    pub unsigned_apo_allowed: bool,
    pub windows_build: u32,
}

pub fn list_mics() -> Vec<Mic> {
    let _ = wasapi::initialize_mta();
    let Ok(en) = wasapi::DeviceEnumerator::new() else { return Vec::new() };
    let default_id = en.get_default_device(&wasapi::Direction::Capture).and_then(|d| d.get_id()).ok();
    let Ok(devices) = en.get_device_collection(&wasapi::Direction::Capture) else { return Vec::new() };
    let mut mics = Vec::new();
    for d in &devices {
        let Ok(d) = d else { continue };
        let Ok(id) = d.get_id() else { continue };
        // id вида {0.0.1.00000000}.{guid}
        let endpoint = id.rsplit('.').next().unwrap_or(&id).to_string();
        let fx = format!(r"{CAPTURE_ROOT}\{endpoint}\FxProperties");
        let installed = [PK_COMPOSITE_SFX, PK_SFX]
            .iter()
            .any(|v| read_strings(&fx, v).iter().any(|s| s.eq_ignore_ascii_case(APO_CLSID)));
        mics.push(Mic {
            endpoint,
            name: d.get_description().or_else(|_| d.get_friendlyname()).unwrap_or_default(),
            adapter: d.get_interface_friendlyname().unwrap_or_default(),
            is_default: default_id.as_deref() == Some(id.as_str()),
            apo_installed: installed,
        });
    }
    mics
}

pub fn system_info() -> SystemInfo {
    let allowed = read_dword(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Audio", "DisableProtectedAudioDG") == Some(1);
    let build = read_string(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    SystemInfo { unsigned_apo_allowed: allowed, windows_build: build }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// REG_SZ или REG_MULTI_SZ как список строк.
fn read_strings(key: &str, value: &str) -> Vec<String> {
    let (k, v) = (wide(key), wide(value));
    let mut buf = vec![0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(k.as_ptr()),
            PCWSTR(v.as_ptr()),
            RRF_RT_REG_SZ | RRF_RT_REG_MULTI_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    }
    .is_ok();
    if !ok {
        return Vec::new();
    }
    buf.truncate(size as usize / 2);
    String::from_utf16_lossy(&buf).split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect()
}

fn read_string(key: &str, value: &str) -> Option<String> {
    read_strings(key, value).into_iter().next()
}

fn read_dword(key: &str, value: &str) -> Option<u32> {
    let (k, v) = (wide(key), wide(value));
    let mut data = 0u32;
    let mut size = 4u32;
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(k.as_ptr()),
            PCWSTR(v.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut _),
            Some(&mut size),
        )
    }
    .is_ok()
    .then_some(data)
}
