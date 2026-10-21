//! Ядро MicMix: захват звука приложений, микшер и связь с APO микрофона.
//! Используется приложением (app/src-tauri) и консольной утилитой micmix-engine.

pub mod apps;
pub mod config;
pub mod engine;
pub mod mics;
pub mod ring;

pub use config::Config;
pub use engine::{Engine, EngineStatus, TONE_ID};
