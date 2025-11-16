//! Razermapper Daemon Library
//!
//! This library provides the core functionality for the razermapper daemon:
//! - Device discovery and management
//! - Macro recording and playback
//! - Input injection via uinput
//! - IPC communication
//! - Security management

use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::RwLock;
use std::collections::HashMap;

pub mod config;
pub mod device;
pub mod macro_engine;
pub mod injector;
pub mod ipc;
pub mod security;

// Re-export common types
pub use razermapper_common::{DeviceInfo, MacroEntry, Profile};

/// DaemonState holds the shared state of the daemon
pub struct DaemonState {
    pub start_time: Instant,
    pub devices: Arc<Mutex<Vec<DeviceInfo>>>,
    pub macros: Arc<Mutex<HashMap<String, MacroEntry>>>,
    pub profiles: Arc<Mutex<HashMap<String, Profile>>>,
    pub macro_engine: Option<Arc<macro_engine::MacroEngine>>,
    pub device_manager: Option<Arc<RwLock<device::DeviceManager>>>,
    pub active_recording: Option<(String, String)>, // (name, device_path)
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            devices: Arc::new(Mutex::new(Vec::new())),
            macros: Arc::new(Mutex::new(HashMap::new())),
            profiles: Arc::new(Mutex::new(HashMap::new())),
            macro_engine: None,
            device_manager: None,
            active_recording: None,
        }
    }
}
