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
    /// Горячая клавиша «включить/выключить этот источник».
    #[serde(default)]
    pub hotkey: Option<String>,
}

/// Звук для кнопки, как в Soundpad.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SoundConfig {
    pub id: String,
    pub name: String,
    /// Копия файла в %APPDATA%\MicMix\sounds — исходный можно удалить.
    pub file: String,
    pub gain: f32,
    #[serde(default)]
    pub hotkey: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub sources: Vec<SourceConfig>,
    pub sounds: Vec<SoundConfig>,
    pub master_gain: f32,
    pub master_muted: bool,
    /// Выключены все приложения (звуки при этом играют).
    pub apps_muted: bool,
    pub sounds_gain: f32,
    /// Слышать звуки самому — в наушниках/колонках по умолчанию.
    pub monitor: bool,
    pub monitor_gain: f32,
    /// Горячая клавиша «выключить/включить весь звук в микрофоне».
    pub mute_hotkey: String,
    /// Горячая клавиша «все приложения вкл/выкл».
    pub apps_hotkey: Option<String>,
    /// Горячая клавиша «остановить все звуки».
    pub stop_hotkey: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            sources: Vec::new(),
            sounds: Vec::new(),
            master_gain: 0.7,
            master_muted: false,
            apps_muted: false,
            sounds_gain: 0.8,
            monitor: true,
            monitor_gain: 0.5,
            mute_hotkey: "Ctrl+Alt+M".into(),
            apps_hotkey: None,
            stop_hotkey: None,
        }
    }
}

/// Папка настроек: %APPDATA%\MicMix.
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("MicMix"))
}

fn path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("config.json"))
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
        // Сначала во временный файл: если компьютер выключится посреди записи, настройки не пропадут.
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(tmp, p).map_err(|e| e.to_string())
    }
}
