use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

// Re-export common dependencies
pub use serde;
pub use bincode;
pub use tokio;
pub use tracing;

// IPC client module
pub mod ipc_client;

/// Information about a connected input device
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub path: PathBuf,
    pub vendor_id: u16,
    pub product_id: u16,
    pub phys: String,
}

impl fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} (VID: {:04X}, PID: {:04X})",
               self.name, self.vendor_id, self.product_id)
    }
}

/// Represents a key combination for macro triggers
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub keys: Vec<u16>, // Key codes
    pub modifiers: Vec<u16>, // Modifier key codes
}

/// Different actions that can be executed by a macro
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    /// Key press with optional key code
    KeyPress(u16),
    /// Key release
    KeyRelease(u16),
    /// Delay in milliseconds
    Delay(u32),
    /// Execute a command
    Execute(String),
    /// Type a string
    Type(String),
    /// Mouse button press
    MousePress(u16),
    /// Mouse button release
    MouseRelease(u16),
    /// Mouse move relative
    MouseMove(i32, i32),
    /// Mouse scroll
    MouseScroll(i32),
}

/// Macro definition with name, trigger combo, and actions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroEntry {
    pub name: String,
    pub trigger: KeyCombo,
    pub actions: Vec<Action>,
    pub device_id: Option<String>, // Optional device restriction
    pub enabled: bool,
}

/// IPC Requests from GUI to Daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// List all available devices
    GetDevices,

    /// Set a macro for a device
    SetMacro {
        device_path: String,
        macro_entry: MacroEntry,
    },

    /// List all configured macros
    ListMacros,

    /// Delete a macro by name
    DeleteMacro {
        name: String,
    },

    /// Reload configuration from disk
    ReloadConfig,

    /// Set LED color for a device
    LedSet {
        device_path: String,
        color: (u8, u8, u8), // RGB
    },

    /// Start recording a macro
    RecordMacro {
        device_path: String,
        name: String,
    },

    /// Stop recording a macro
    StopRecording,

    /// Test a macro execution
    TestMacro {
        name: String,
    },

    /// Get daemon status and version
    GetStatus,

    /// Save current macros to a profile
    SaveProfile {
        name: String,
    },

    /// Load macros from a profile
    LoadProfile {
        name: String,
    },

    /// List available profiles
    ListProfiles,

    /// Delete a profile
    DeleteProfile {
        name: String,
    },

    /// Generate an authentication token
    GenerateToken {
        client_id: String,
    },

    /// Authenticate with a token
    Authenticate {
        token: String,
    },

    /// Execute a macro by name
    ExecuteMacro {
        name: String,
    },

    /// Grab a device exclusively for input interception
    GrabDevice {
        device_path: String,
    },

    /// Release exclusive access to a device
    UngrabDevice {
        device_path: String,
    },
}

/// Status information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusInfo {
    pub version: String,
    pub uptime_seconds: u64,
    pub devices_count: usize,
    pub macros_count: usize,
}

/// IPC Responses from Daemon to GUI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    /// List of discovered devices
    Devices(Vec<DeviceInfo>),

    /// List of configured macros
    Macros(Vec<MacroEntry>),

    /// Acknowledgment of successful operation
    Ack,

    /// Status information
    Status {
        version: String,
        uptime_seconds: u64,
        devices_count: usize,
        macros_count: usize,
    },

    /// Notification that recording has started
    RecordingStarted {
        device_path: String,
        name: String,
    },

    /// Notification that recording has stopped
    RecordingStopped {
        macro_entry: MacroEntry,
    },

    /// List of available profiles
    Profiles(Vec<String>),

    /// Profile load confirmation
    ProfileLoaded {
        name: String,
        macros_count: usize,
    },

    /// Profile save confirmation
    ProfileSaved {
        name: String,
        macros_count: usize,
    },

    /// Error response
    Error(String),

    /// Authentication token
    Token(String),

    /// Authentication successful
    Authenticated,
}

/// Profile structure for organizing macros
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub macros: std::collections::HashMap<String, MacroEntry>,
}

/// Serialization helpers for the IPC protocol
pub fn serialize<T: Serialize>(msg: &T) -> Vec<u8> {
    bincode::serialize(msg).unwrap_or_else(|e| {
        tracing::error!("Failed to serialize message: {:?}", e);
        Vec::new()
    })
}

pub fn deserialize<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, bincode::Error> {
    bincode::deserialize(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_serialization() {
        let request = Request::GetDevices;
        let serialized = serialize(&request);
        let deserialized: Request = deserialize(&serialized).unwrap();
        assert!(matches!(deserialized, Request::GetDevices));
    }

    #[test]
    fn test_macro_entry_serialization() {
        let macro_entry = MacroEntry {
            name: "Test Macro".to_string(),
            trigger: KeyCombo {
                keys: vec![30, 40], // A and D keys
                modifiers: vec![29], // Ctrl key
            },
            actions: vec![
                Action::KeyPress(30),
                Action::Delay(100),
                Action::KeyRelease(30),
            ],
            device_id: Some("test_device".to_string()),
            enabled: true,
        };

        let serialized = serialize(&macro_entry);
        let deserialized: MacroEntry = deserialize(&serialized).unwrap();
        assert_eq!(deserialized.name, "Test Macro");
        assert_eq!(deserialized.trigger.keys, vec![30, 40]);
    }
}
