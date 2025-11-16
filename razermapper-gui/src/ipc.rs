//! IPC module for the razermapper GUI
//!
//! This module provides a simplified interface for the GUI to communicate
//! with the razermapper daemon using the common IPC client.

use razermapper_common::{ipc_client, DeviceInfo, MacroEntry, Request, Response};
use std::path::PathBuf;
// Import removed as it's not used

/// Simplified IPC client for the GUI
pub struct GuiIpcClient {
    socket_path: PathBuf,
}

impl GuiIpcClient {
    /// Create a new GUI IPC client with the specified socket path
    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    /// Connect to the daemon
    pub async fn connect(&self) -> Result<(), String> {
        match ipc_client::is_daemon_running(Some(&self.socket_path)).await {
            true => Ok(()),
            false => Err("Daemon is not running".to_string()),
        }
    }

    /// Get list of available devices
    pub async fn get_devices(&self) -> Result<Vec<DeviceInfo>, String> {
        let request = Request::GetDevices;
        match ipc_client::send_to_path(&request, &self.socket_path).await {
            Ok(Response::Devices(devices)) => Ok(devices),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Failed to get devices: {}", e)),
        }
    }

    /// Get list of configured macros
    pub async fn list_macros(&self) -> Result<Vec<MacroEntry>, String> {
        let request = Request::ListMacros;
        match ipc_client::send_to_path(&request, &self.socket_path).await {
            Ok(Response::Macros(macros)) => Ok(macros),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Failed to list macros: {}", e)),
        }
    }

    /// Start recording a macro for a device
    pub async fn start_recording_macro(&self, device_path: &str, name: &str) -> Result<(), String> {
        let request = Request::RecordMacro {
            device_path: device_path.to_string(),
            name: name.to_string(),
        };
        match ipc_client::send_to_path(&request, &self.socket_path).await {
            Ok(Response::RecordingStarted { .. }) => Ok(()),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Failed to start recording: {}", e)),
        }
    }

    /// Stop recording a macro
    pub async fn stop_recording_macro(&self) -> Result<MacroEntry, String> {
        let request = Request::StopRecording;
        match ipc_client::send_to_path(&request, &self.socket_path).await {
            Ok(Response::RecordingStopped { macro_entry }) => Ok(macro_entry),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Failed to stop recording: {}", e)),
        }
    }

    /// Delete a macro by name
    pub async fn delete_macro(&self, name: &str) -> Result<(), String> {
        let request = Request::DeleteMacro {
            name: name.to_string(),
        };
        match ipc_client::send_to_path(&request, &self.socket_path).await {
            Ok(Response::Ack) => Ok(()),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Failed to delete macro: {}", e)),
        }
    }

    /// Test a macro execution
    pub async fn test_macro(&self, name: &str) -> Result<(), String> {
        let request = Request::TestMacro {
            name: name.to_string(),
        };
        match ipc_client::send_to_path(&request, &self.socket_path).await {
            Ok(Response::Ack) => Ok(()),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Failed to test macro: {}", e)),
        }
    }

    /// Save current macros to a profile
    pub async fn save_profile(&self, name: &str) -> Result<(String, usize), String> {
        let request = Request::SaveProfile {
            name: name.to_string(),
        };
        match ipc_client::send_to_path(&request, &self.socket_path).await {
            Ok(Response::ProfileSaved { name, macros_count }) => Ok((name, macros_count)),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Failed to save profile: {}", e)),
        }
    }

    /// Load macros from a profile
    pub async fn load_profile(&self, name: &str) -> Result<(String, usize), String> {
        let request = Request::LoadProfile {
            name: name.to_string(),
        };
        match ipc_client::send_to_path(&request, &self.socket_path).await {
            Ok(Response::ProfileLoaded { name, macros_count }) => Ok((name, macros_count)),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Failed to load profile: {}", e)),
        }
    }

    /// Grab a device exclusively for input interception
    pub async fn grab_device(&self, device_path: &str) -> Result<(), String> {
        let request = Request::GrabDevice {
            device_path: device_path.to_string(),
        };
        match ipc_client::send_to_path(&request, &self.socket_path).await {
            Ok(Response::Ack) => Ok(()),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Failed to grab device: {}", e)),
        }
    }

    /// Release exclusive access to a device
    pub async fn ungrab_device(&self, device_path: &str) -> Result<(), String> {
        let request = Request::UngrabDevice {
            device_path: device_path.to_string(),
        };
        match ipc_client::send_to_path(&request, &self.socket_path).await {
            Ok(Response::Ack) => Ok(()),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Failed to ungrab device: {}", e)),
        }
    }
}

/// Type alias for the IPC client used in the GUI
pub type IpcClient = GuiIpcClient;
