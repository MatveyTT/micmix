//! Настройки, которые переживают перезапуск: %APPDATA%\MicMix\config.json.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceConfig {
    /// Имя exe в нижнем регистре — по нему источник находится после перезапуска приложения.
    pub exe: String,
    pub name: String,
    pub gain: f32,
    pub muted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub sources: Vec<SourceConfig>,
    pub master_gain: f32,
    pub master_muted: bool,
    /// Горячая клавиша «выключить/включить звук в микрофоне».
    pub mute_hotkey: String,
}

impl Default for Config {
    fn default() -> Self {
        Config { sources: Vec::new(), master_gain: 0.7, master_muted: false, mute_hotkey: "Ctrl+Shift+M".into() }
    }
}

fn path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("MicMix").join("config.json"))
}

impl Config {
    pub fn load() -> Config {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            // Блокнот и PowerShell 5.1 пишут UTF-8 с BOM — serde_json его не принимает.
            .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = path().ok_or("нет %APPDATA%")?;
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(p, json).map_err(|e| e.to_string())
    }
}
