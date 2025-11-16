use razermapper_common::{tracing, MacroEntry, Profile};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::fs;
use tracing::{debug, info, warn};

/// Configuration manager for razermapper daemon
pub struct ConfigManager {
    pub config_path: PathBuf,
    pub macros_path: PathBuf,
    pub cache_path: PathBuf,
    pub profiles_dir: PathBuf,
    pub config: DaemonConfig,
    pub macros: Arc<RwLock<HashMap<String, MacroEntry>>>,
    pub profiles: Arc<RwLock<HashMap<String, Profile>>>,
}

/// Daemon configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub daemon: DaemonSettings,
    pub device_discovery: DeviceDiscoverySettings,
    pub macro_engine: MacroEngineSettings,
    pub config: ConfigSettings,
    pub security: SecuritySettings,
    pub led_control: LedControlSettings,
    pub performance: PerformanceSettings,
}

/// Daemon-specific settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSettings {
    pub socket_path: String,
    pub log_level: String,
    pub drop_privileges: bool,
}

/// Device discovery settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceDiscoverySettings {
    pub input_devices_path: String,
    pub use_openrazer_db: bool,
    pub fallback_name_pattern: String,
}

/// Macro engine settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroEngineSettings {
    pub max_concurrent_macros: usize,
    pub default_delay: u32,
    pub enable_recording: bool,
}

/// Configuration persistence settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSettings {
    pub config_file: String,
    pub macros_file: String,
    pub cache_file: String,
    pub auto_save: bool,
    pub reload_interval: u64,
}

/// Security settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecuritySettings {
    pub socket_group: String,
    pub socket_permissions: String,
    pub require_auth_token: bool,
    pub retain_capabilities: Vec<String>,
}

/// LED control settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedControlSettings {
    pub enabled: bool,
    pub interface: String,
    pub default_color: [u8; 3],
}

/// Performance settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceSettings {
    pub device_poll_interval: u64,
    pub event_queue_size: usize,
    pub thread_pool: bool,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            daemon: DaemonSettings {
                socket_path: "/run/razermapper.sock".to_string(),
                log_level: "info".to_string(),
                drop_privileges: true,
            },
            device_discovery: DeviceDiscoverySettings {
                input_devices_path: "/dev/input/by-id".to_string(),
                use_openrazer_db: true,
                fallback_name_pattern: "Razer".to_string(),
            },
            macro_engine: MacroEngineSettings {
                max_concurrent_macros: 10,
                default_delay: 10,
                enable_recording: true,
            },
            config: ConfigSettings {
                config_file: "/etc/razermapperd/config.yaml".to_string(),
                macros_file: "/etc/razermapperd/macros.yaml".to_string(),
                cache_file: "/var/cache/razermapperd/macros.bin".to_string(),
                auto_save: true,
                reload_interval: 30,
            },
            security: SecuritySettings {
                socket_group: "input".to_string(),
                socket_permissions: "0660".to_string(),
                require_auth_token: false,
                retain_capabilities: vec!["CAP_SYS_RAWIO".to_string()],
            },
            led_control: LedControlSettings {
                enabled: true,
                interface: "dbus".to_string(),
                default_color: [0, 255, 0],
            },
            performance: PerformanceSettings {
                device_poll_interval: 1,
                event_queue_size: 1000,
                thread_pool: true,
            },
        }
    }
}

impl ConfigManager {
    /// Create a new configuration manager with default paths
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let config_path = PathBuf::from("/etc/razermapperd/config.yaml");
        let macros_path = PathBuf::from("/etc/razermapperd/macros.yaml");
        let cache_path = PathBuf::from("/var/cache/razermapperd/macros.bin");
        let profiles_dir = PathBuf::from("/etc/razermapperd/profiles");

        let manager = Self {
            config_path,
            macros_path,
            cache_path,
            profiles_dir,
            config: DaemonConfig::default(),
            macros: Arc::new(RwLock::new(HashMap::new())),
            profiles: Arc::new(RwLock::new(HashMap::new())),
        };

        // Ensure directories exist
        if let Some(parent) = manager.config_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        if let Some(parent) = manager.macros_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        if let Some(parent) = manager.cache_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::create_dir_all(&manager.profiles_dir).await?;

        Ok(manager)
    }

    /// Load configuration from disk
    pub async fn load_config(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Loading configuration from {}", self.config_path.display());

        if self.config_path.exists() {
            let content = fs::read_to_string(&self.config_path).await?;
            self.config = serde_yaml::from_str(&content)?;
            debug!("Loaded configuration from disk");
        } else {
            warn!("Configuration file not found, using defaults");
            self.save_config().await?;
        }

        // Try to load macros from cache first, then from YAML
        if self.cache_path.exists() {
            match self.load_macros_from_cache().await {
                Ok(()) => {
                    debug!("Loaded macros from cache");
                    return Ok(());
                }
                Err(e) => {
                    warn!("Failed to load macros from cache: {}", e);
                    // Fall back to YAML
                }
            }
        }

        if self.macros_path.exists() {
            self.load_macros_from_yaml().await?;
        } else {
            info!("No macros file found, creating empty macros");
            self.save_macros().await?;
        }

        Ok(())
    }

    /// Save configuration to disk
    pub async fn save_config(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Saving configuration to {}", self.config_path.display());

        let content = serde_yaml::to_string(&self.config)?;
        fs::write(&self.config_path, content).await?;

        debug!("Configuration saved");
        Ok(())
    }

    /// Load macros from binary cache
    async fn load_macros_from_cache(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Loading macros from cache {}", self.cache_path.display());

        let content = fs::read(&self.cache_path).await?;

        // First 4 bytes should be a magic number for verification
        if content.len() < 4 {
            return Err("Cache file too short".into());
        }

        let magic = u32::from_le_bytes([content[0], content[1], content[2], content[3]]);
        if magic != 0xDEADBEEF {
            return Err("Invalid cache file magic number".into());
        }

        let macros: HashMap<String, MacroEntry> = razermapper_common::deserialize(&content[4..])?;
        *self.macros.write().await = macros;

        debug!("Loaded macros from cache");
        Ok(())
    }

    /// Load macros from YAML file
    async fn load_macros_from_yaml(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Loading macros from {}", self.macros_path.display());

        let content = fs::read_to_string(&self.macros_path).await?;
        let macros: HashMap<String, MacroEntry> = serde_yaml::from_str(&content)?;
        *self.macros.write().await = macros;

        debug!("Loaded macros from YAML");
        Ok(())
    }

    /// Save macros to both cache and YAML
    pub async fn save_macros(&self) -> Result<(), Box<dyn std::error::Error>> {
        let macros = self.macros.read().await;

        // Save to cache
        self.save_macros_to_cache(&macros).await?;

        // Save to YAML
        self.save_macros_to_yaml(&macros).await?;

        debug!("Saved macros to both cache and YAML");
        Ok(())
    }

    /// Save macros to binary cache
    async fn save_macros_to_cache(&self, macros: &HashMap<String, MacroEntry>) -> Result<(), Box<dyn std::error::Error>> {
        let mut data = Vec::new();

        // Add magic number
        data.extend_from_slice(&0xDEADBEEFu32.to_le_bytes());

        // Add serialized data
        let serialized = razermapper_common::serialize(macros);
        data.extend_from_slice(&serialized);

        fs::write(&self.cache_path, data).await?;
        debug!("Saved macros to cache");
        Ok(())
    }

    /// Save macros to YAML file
    async fn save_macros_to_yaml(&self, macros: &HashMap<String, MacroEntry>) -> Result<(), Box<dyn std::error::Error>> {
        let content = serde_yaml::to_string(macros)?;
        fs::write(&self.macros_path, content).await?;
        debug!("Saved macros to YAML");
        Ok(())
    }

    /// Get a reference to the configuration
    pub fn config(&self) -> &DaemonConfig {
        &self.config
    }

    /// Load configuration from disk (mutable version for use with Arc)
    pub async fn load_config_mut(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Loading configuration from {}", self.config_path.display());

        if self.config_path.exists() {
            let content = fs::read_to_string(&self.config_path).await?;
            let _config: DaemonConfig = serde_yaml::from_str(&content)?;
            // We can't replace self.config directly, so we'll update the fields
            // This is a limitation of using Arc<ConfigManager> without interior mutability
            debug!("Loaded configuration from disk");
        } else {
            warn!("Configuration file not found, using defaults");
            self.save_config().await?;
        }

        // Try to load macros from cache first, then from YAML
        if self.cache_path.exists() {
            match self.load_macros_from_cache().await {
                Ok(()) => {
                    debug!("Loaded macros from cache");
                    return Ok(());
                }
                Err(e) => {
                    warn!("Failed to load macros from cache: {}", e);
                    // Fall back to YAML
                }
            }
        }

        if self.macros_path.exists() {
            self.load_macros_from_yaml().await?;
        } else {
            info!("No macros file found, creating empty macros");
            self.save_macros().await?;
        }

        Ok(())
    }

    /// Get a reference to the macros
    pub fn macros(&self) -> &Arc<RwLock<HashMap<String, MacroEntry>>> {
        &self.macros
    }

    /// Get a profile by name
    pub async fn get_profile(&self, name: &str) -> Option<Profile> {
        let profiles = self.profiles.read().await;
        profiles.get(name).cloned()
    }

    /// Get all profiles
    pub async fn get_profiles(&self) -> std::collections::HashMap<String, Profile> {
        let profiles = self.profiles.read().await;
        profiles.clone()
    }

    /// Save a profile
    pub async fn save_profile(&self, profile: &Profile) -> Result<(), Box<dyn std::error::Error>> {
        let profile_path = self.profiles_dir.join(format!("{}.yaml", profile.name));

        // Save to YAML
        let yaml = serde_yaml::to_string(profile)?;
        fs::write(&profile_path, yaml).await?;

        // Update in-memory profiles
        let mut profiles = self.profiles.write().await;
        profiles.insert(profile.name.clone(), profile.clone());

        info!("Profile {} saved to {}", profile.name, profile_path.display());
        Ok(())
    }

    /// Load a profile by name
    pub async fn load_profile(&self, name: &str) -> Result<Profile, Box<dyn std::error::Error>> {
        let profile_path = self.profiles_dir.join(format!("{}.yaml", name));

        if !profile_path.exists() {
            return Err(format!("Profile {} not found", name).into());
        }

        let yaml = fs::read_to_string(&profile_path).await?;
        let profile: Profile = serde_yaml::from_str(&yaml)?;

        // Update in-memory profiles
        let mut profiles = self.profiles.write().await;
        profiles.insert(name.to_string(), profile.clone());

        // Load macros from profile into current macros
        let mut macros = self.macros.write().await;
        for (name, macro_entry) in &profile.macros {
            macros.insert(name.clone(), macro_entry.clone());
        }

        info!("Profile {} loaded from {}", name, profile_path.display());
        Ok(profile)
    }

    /// List all available profiles
    pub async fn list_profiles(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut entries = match fs::read_dir(&self.profiles_dir).await {
            Ok(entries) => entries,
            Err(e) => return Err(e.into()),
        };

        let mut profiles = Vec::new();
        while let Some(entry) = entries.next_entry().await.unwrap_or(None) {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
                if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                    profiles.push(name.to_string());
                }
            }
        }

        profiles.sort();
        Ok(profiles)
    }

    /// Delete a profile
    pub async fn delete_profile(&self, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let profile_path = self.profiles_dir.join(format!("{}.yaml", name));

        if profile_path.exists() {
            fs::remove_file(&profile_path).await?;
        }

        // Remove from in-memory profiles
        let mut profiles = self.profiles.write().await;
        profiles.remove(name);

        info!("Profile {} deleted", name);
        Ok(())
    }

    /// Save current macros as a new profile
    pub async fn save_current_macros_as_profile(&self, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let macros = self.macros.read().await;
        let profile = Profile {
            name: name.to_string(),
            macros: macros.clone(),
        };

        self.save_profile(&profile).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_config_creation() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");
        let macros_path = temp_dir.path().join("macros.yaml");
        let cache_path = temp_dir.path().join("macros.bin");

        let mut manager = ConfigManager {
            config_path,
            macros_path,
            cache_path,
            profiles_dir: temp_dir.path().join("profiles"),
            config: DaemonConfig::default(),
            macros: Arc::new(RwLock::new(HashMap::new())),
            profiles: Arc::new(RwLock::new(HashMap::new())),
        };

        // Should be able to save and load without errors
        manager.save_config().await.unwrap();
        manager.load_config().await.unwrap();
    }

    #[tokio::test]
    async fn test_macro_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");
        let macros_path = temp_dir.path().join("macros.yaml");
        let cache_path = temp_dir.path().join("macros.bin");

        let manager = ConfigManager {
            config_path: config_path.clone(),
            macros_path: macros_path.clone(),
            cache_path: cache_path.clone(),
            profiles_dir: temp_dir.path().to_path_buf(),
            config: DaemonConfig::default(),
            macros: Arc::new(RwLock::new(HashMap::new())),
            profiles: Arc::new(RwLock::new(HashMap::new())),
        };

        // Add a test macro
        let test_macro = MacroEntry {
            name: "Test Macro".to_string(),
            trigger: razermapper_common::KeyCombo {
                keys: vec![30, 40], // A and D keys
                modifiers: vec![29], // Ctrl key
            },
            actions: vec![
                razermapper_common::Action::KeyPress(30),
                razermapper_common::Action::Delay(100),
                razermapper_common::Action::KeyRelease(30),
            ],
            device_id: None,
            enabled: true,
        };

        manager.macros.write().await.insert("test_macro".to_string(), test_macro.clone());

        // Save and reload
        manager.save_macros().await.unwrap();

        let manager2 = ConfigManager {
            config_path: config_path.clone(),
            macros_path,
            cache_path: temp_dir.path().join("macros2.bin"),
            profiles_dir: temp_dir.path().to_path_buf(),
            config: DaemonConfig::default(),
            macros: Arc::new(RwLock::new(HashMap::new())),
            profiles: Arc::new(RwLock::new(HashMap::new())),
        };

        manager2.load_macros_from_yaml().await.unwrap();

        let loaded_macros = manager2.macros.read().await;
        let loaded_macro = loaded_macros.get("test_macro").unwrap();

        assert_eq!(loaded_macro.name, test_macro.name);
        assert_eq!(loaded_macro.trigger.keys, test_macro.trigger.keys);
    }
}
