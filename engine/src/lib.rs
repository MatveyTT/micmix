//! Ядро MicMix: захват звука приложений, звуки-кнопки, микшер и связь с APO микрофона.
//! Используется приложением (app/src-tauri) и консольной утилитой micmix-engine.

pub mod apps;
pub mod config;
pub mod engine;
pub mod mics;
pub mod monitor;
pub mod ring;
pub mod sounds;
pub mod tab;

pub use config::{Config, SoundConfig, SourceConfig};
pub use engine::{Engine, EngineStatus, TAB_ID, TONE_ID};
