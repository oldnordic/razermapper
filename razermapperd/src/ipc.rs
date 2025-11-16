use razermapper_common::{tracing, serialize, deserialize, Request, Response};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;
use tokio::task;
use tracing::{debug, error, info, warn};

use crate::macro_engine;
use crate::config;
use crate::injector;
use crate::security;
// crate::device used via DaemonState.device_manager

/// IPC server for handling communication with GUI clients
pub struct IpcServer {
    socket_path: String,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    macro_engine: Option<Arc<macro_engine::MacroEngine>>,
    injector: Option<Arc<RwLock<dyn injector::Injector + Send + Sync>>>,
    security_manager: Option<Arc<RwLock<security::SecurityManager>>>,
}


impl IpcServer {
    /// Create a new IPC server with the specified socket path
    pub fn new<P: AsRef<Path>>(socket_path: P) -> Result<Self, std::io::Error> {
        let path = socket_path.as_ref().to_string_lossy().to_string();

        // Remove any existing socket file
        if Path::new(&path).exists() {
            std::fs::remove_file(&path)?;
        }

        Ok(Self {
            socket_path: path,
            shutdown_tx: None,
            macro_engine: None,
            injector: None,
            security_manager: None,
        })
    }

    /// Start the IPC server with the provided daemon state
    pub async fn start(&mut self,
            state: Arc<RwLock<crate::DaemonState>>,
            macro_engine: Arc<macro_engine::MacroEngine>,
            injector: Arc<RwLock<dyn injector::Injector + Send + Sync>>,
            config_manager: Arc<config::ConfigManager>,
            security_manager: Arc<RwLock<security::SecurityManager>>
        ) -> Result<(), Box<dyn std::error::Error>> {
        info!("Starting IPC server at {}", self.socket_path);

        // Store references to macro engine and injector
        self.macro_engine = Some(macro_engine);
        self.injector = Some(injector.clone());
        self.security_manager = Some(security_manager.clone());

        // Create Unix listener
        let listener = UnixListener::bind(&self.socket_path)?;

        // Set socket permissions using the security manager
        {
            let security = security_manager.read().await;
            if let Err(e) = security.set_socket_permissions(&self.socket_path) {
                warn!("Failed to set socket permissions: {}", e);
                // Continue anyway, as the daemon should still work even if permissions aren't ideal
            }
        }

        // Create shutdown channel
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        // Spawn the main server loop
        let _state_clone = Arc::clone(&state);
        // Clone references before moving into task
        let macro_engine = self.macro_engine.as_ref().unwrap().clone();
        let injector = self.injector.as_ref().unwrap().clone();

        task::spawn(async move {
            loop {
                tokio::select! {
                    // Accept new connections
                    connection = listener.accept() => {
                        match connection {
                            Ok((stream, _)) => {
                                debug!("New client connected");
                                let state = Arc::clone(&state);
                                let macro_engine = Arc::clone(&macro_engine);
                                let injector = Arc::clone(&injector);
                                let config_manager = Arc::clone(&config_manager);
                                let security_manager = Arc::clone(&security_manager);
                                task::spawn(async move {
                                    if let Err(e) = handle_client(
                                        stream,
                                        state,
                                        macro_engine,
                                        injector,
                                        config_manager,
                                        security_manager
                                    ).await {
                                        error!("Error handling client: {}", e);
                                    }
                                });
                            }
                            Err(e) => {
                                error!("Error accepting connection: {}", e);
                            }
                        }
                    }
                    // Handle shutdown signal
                    _ = &mut shutdown_rx => {
                        info!("Shutting down IPC server");
                        break;
                    }
                }
            }
        });

        // Note: We can't modify self.executing here because we're in a spawned task
        // In a real implementation, we would use a channel or other communication method
        debug!("Macro execution completed");
        Ok(())
    }

    /// Shutdown the IPC server
    pub async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Shutting down IPC server");

        // Send shutdown signal to the server task
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }

        // Remove the socket file
        if Path::new(&self.socket_path).exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        Ok(())
    }
}

/// Handle a client connection
pub async fn handle_client(
    mut stream: UnixStream,
    state: Arc<RwLock<crate::DaemonState>>,
    macro_engine: Arc<macro_engine::MacroEngine>,
    injector: Arc<RwLock<dyn injector::Injector + Send + Sync>>,
    config_manager: Arc<config::ConfigManager>,
    security_manager: Arc<RwLock<security::SecurityManager>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Set a reasonable timeout for operations
    // Note: set_keepalive is not available on UnixStream in this version of tokio
    // stream.set_keepalive(Some(std::time::Duration::from_secs(30)))?;

    // Read message length first
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let msg_len = u32::from_le_bytes(len_buf) as usize;

    // Validate message length to prevent excessive memory usage
    if msg_len > 1024 * 1024 { // 1MB max message size
        warn!("Received oversized message: {} bytes", msg_len);
        return Err("Message too large".into());
    }

    // Read the actual message
    let mut msg_buf = vec![0u8; msg_len];
    stream.read_exact(&mut msg_buf).await?;

    // Deserialize the request
    let request: Request = deserialize(&msg_buf)?;
    debug!("Received request: {:?}", request);

    // Check authentication if token auth is enabled
    let auth_required = cfg!(feature = "token-auth");

    if auth_required {
        // Handle authentication request
        if let Request::Authenticate { token } = &request {
            let security = security_manager.read().await;
            if security.validate_auth_token(token).await {
                debug!("Authentication successful");
                let response = Response::Authenticated;
                let response_bytes = serialize(&response);

                // Send the response length first
                let len = response_bytes.len() as u32;
                stream.write_all(&len.to_le_bytes()).await?;

                // Send the response
                stream.write_all(&response_bytes).await?;
                stream.flush().await?;

                return Ok(());
            } else {
                debug!("Authentication failed");
                let response = Response::Error("Invalid authentication token".to_string());
                let response_bytes = serialize(&response);

                // Send the response length first
                let len = response_bytes.len() as u32;
                stream.write_all(&len.to_le_bytes()).await?;

                // Send the response
                stream.write_all(&response_bytes).await?;
                stream.flush().await?;

                return Ok(());
            }
        }
        // Allow GenerateToken without authentication
        else if !matches!(request, Request::GenerateToken { .. }) {
            debug!("Authentication required but not provided");
            let response = Response::Error("Authentication required".to_string());
            let response_bytes = serialize(&response);

            // Send the response length first
            let len = response_bytes.len() as u32;
            stream.write_all(&len.to_le_bytes()).await?;

            // Send the response
            stream.write_all(&response_bytes).await?;
            stream.flush().await?;

            return Ok(());
        }
    }

    // Process the request and generate a response
    let response = handle_request(
        request,
        Arc::clone(&state),
        Arc::clone(&macro_engine),
        Arc::clone(&injector),
        Arc::clone(&config_manager),
        Arc::clone(&security_manager)
    ).await;
    debug!("Sending response: {:?}", response);

    // Serialize the response
    let response_bytes = serialize(&response);

    // Send the response length first
    let len = response_bytes.len() as u32;
    stream.write_all(&len.to_le_bytes()).await?;

    // Send the response
    stream.write_all(&response_bytes).await?;
    stream.flush().await?;

    Ok(())
}

/// Process a request and generate a response
async fn handle_request(
    request: Request,
    state: Arc<RwLock<crate::DaemonState>>,
    macro_engine: Arc<macro_engine::MacroEngine>,
    _injector: Arc<RwLock<dyn injector::Injector + Send + Sync>>,
    config_manager: Arc<config::ConfigManager>,
    security_manager: Arc<RwLock<security::SecurityManager>>,
) -> Response {
    match request {
        Request::GenerateToken { client_id } => {
            debug!("Generating token for client: {}", client_id);
            let security = security_manager.read().await;
            match security.generate_auth_token().await {
                Ok(token) => Response::Token(token),
                Err(e) => Response::Error(format!("Failed to generate token: {}", e)),
            }
        }
        Request::Authenticate { token } => {
            let security = security_manager.read().await;
            if security.validate_auth_token(&token).await {
                Response::Authenticated
            } else {
                Response::Error("Invalid authentication token".to_string())
            }
        }
        Request::GetDevices => {
            let state = state.read().await;
            let devices = state.devices.lock().unwrap().clone();
            Response::Devices(devices)
        }
        Request::ListMacros => {
            let state = state.read().await;
            let macros = state.macros.lock().unwrap().values().cloned().collect();
            return Response::Macros(macros);
        }
        Request::SetMacro { device_path, macro_entry } => {
            let state = state.write().await;

            // Check if the device exists
            let devices = state.devices.lock().unwrap();
            let device_exists = devices.iter().any(|d| d.path.to_string_lossy() == device_path);
            if !device_exists {
                return Response::Error(format!("Device not found: {}", device_path));
            }

            // Add or update the macro
            let mut macros = state.macros.lock().unwrap();
            macros.insert(macro_entry.name.clone(), macro_entry);

            return Response::Ack;
        }
        Request::DeleteMacro { name } => {
            let state = state.write().await;

            // Find and remove the macro
            let mut macros = state.macros.lock().unwrap();
            let original_len = macros.len();
            macros.remove(&name);

            if macros.len() == original_len {
                return Response::Error(format!("Macro not found: {}", name));
            } else {
                return Response::Ack;
            }
        }
        Request::ReloadConfig => {
            // This would trigger a config reload in a real implementation
            info!("Config reload requested");
            return Response::Ack;
        }
        Request::LedSet { device_path, color } => {
            // This would set LED colors in a real implementation
            info!("LED set request for {}: {:?}", device_path, color);
            return Response::Ack;
        }
        Request::RecordMacro { device_path, name } => {
            // Start macro recording
            match macro_engine.start_recording(name.clone(), device_path.clone()).await.map_err(|e| format!("Failed to start recording: {}", e)) {
                Ok(_) => {
                    info!("Macro recording started for {} on {}", name, device_path);

                    // Store the recording in the daemon state for access by input event handlers
                    let mut state = state.write().await;
                    state.active_recording = Some((name.clone(), device_path.clone()));

                    return Response::RecordingStarted { device_path, name };
                }
                Err(e) => {
                    error!("Failed to start recording: {}", e);
                    return Response::Error(format!("Failed to start recording: {}", e));
                }
            }
        }
        Request::StopRecording => {
            // Stop macro recording
            match macro_engine.stop_recording().await.map_err(|e| format!("Failed to stop recording: {}", e)) {
                Ok(Some(macro_entry)) => {
                    let macro_name = macro_entry.name.clone();
                    info!("Macro recording stopped: {}", macro_name);

                    // Update the daemon state to remove the active recording
                    let mut state = state.write().await;
                    state.active_recording = None;

                    // Add the macro to the daemon state
                    let mut macros = state.macros.lock().unwrap();
                    macros.insert(macro_entry.name.clone(), macro_entry.clone());
                    drop(macros);

                    return Response::RecordingStopped { macro_entry };
                }
                Ok(None) => {
                    // Update the daemon state to remove the active recording
                    let mut state = state.write().await;
                    state.active_recording = None;

                    return Response::Error("Recording stopped but no macro was created".to_string());
                }
                Err(e) => {
                    error!("Failed to stop recording: {}", e);
                    return Response::Error(format!("Failed to stop recording: {}", e));
                }
            }
        }
        Request::TestMacro { name } => {
            // Test macro execution
            info!("Test macro execution requested: {}", name);
            // Get the macro to execute
            let macro_to_execute = {
                let macros = macro_engine.list_macros().await;
                macros.into_iter().find(|m| m.name == name)
            };

            match macro_to_execute {
                Some(macro_entry) => {
                    // Execute macro using macro engine
                    debug!("Macro execution requested: {}", macro_entry.name);
                    match macro_engine.execute_macro(macro_entry.clone()).await {
                        Ok(_) => {
                            info!("Successfully executed macro: {}", macro_entry.name);
                            Response::Ack
                        }
                        Err(e) => {
                            error!("Failed to execute macro '{}': {}", macro_entry.name, e);
                            Response::Error(format!("Failed to execute macro '{}': {}", macro_entry.name, e))
                        }
                    }
                }
                None => {
                    error!("Macro not found: {}", name);
                    Response::Error(format!("Macro not found: {}", name))
                }
            }
        }
        Request::ExecuteMacro { name } => {
            // Execute macro by name
            info!("Execute macro requested: {}", name);
            // Get the macro to execute
            let macro_to_execute = {
                let macros = macro_engine.list_macros().await;
                macros.into_iter().find(|m| m.name == name)
            };

            match macro_to_execute {
                Some(macro_entry) => {
                    // Use execute_macro method instead of manually executing actions
                    match macro_engine.execute_macro(macro_entry.clone()).await {
                        Ok(_) => {
                            info!("Successfully executed macro: {}", macro_entry.name);
                            Response::Ack
                        }
                        Err(e) => {
                            error!("Failed to execute macro '{}': {}", macro_entry.name, e);
                            Response::Error(format!("Failed to execute macro '{}': {}", macro_entry.name, e))
                        }
                    }
                }
                None => {
                    error!("Macro not found: {}", name);
                    Response::Error(format!("Macro not found: {}", name))
                }
            }
        }
        Request::GetStatus => {
            let state = state.read().await;
            let devices_count = state.devices.lock().unwrap().len();
            let macros_count = state.macros.lock().unwrap().len();
            return Response::Status {
                version: "0.1.0".to_string(),
                uptime_seconds: 0, // Would be calculated in real implementation
                devices_count,
                macros_count,
            };
        }
        Request::SaveProfile { name } => {
            // Save current macros as a profile
            let macros_count = {
                let state_guard = state.read().await;
                let macros = state_guard.macros.lock().unwrap();
                let count = macros.len();
                drop(macros);
                drop(state_guard);
                count
            };

            match config_manager.save_current_macros_as_profile(&name).await {
                Ok(_) => {
                    info!("Profile {} saved", name);
                    return Response::ProfileSaved {
                        name,
                        macros_count,
                    };
                }
                Err(e) => {
                    error!("Failed to save profile: {}", e);
                    return Response::Error(format!("Failed to save profile: {}", e));
                }
            }
        }
        Request::LoadProfile { name } => {
            // Load a profile
            match config_manager.load_profile(&name).await {
                Ok(profile) => {
                    info!("Profile {} loaded", name);
                    return Response::ProfileLoaded {
                        name,
                        macros_count: profile.macros.len()
                    };
                }
                Err(e) => {
                    error!("Failed to load profile: {}", e);
                    return Response::Error(format!("Failed to load profile: {}", e));
                }
            }
        }
        Request::ListProfiles => {
            // List available profiles
            match config_manager.list_profiles().await {
                Ok(profiles) => return Response::Profiles(profiles),
                Err(e) => {
                    error!("Failed to list profiles: {}", e);
                    return Response::Error(format!("Failed to list profiles: {}", e));
                }
            }
        }
        Request::DeleteProfile { name } => {
            // Delete a profile
            match config_manager.delete_profile(&name).await {
                Ok(_) => {
                    info!("Profile {} deleted", name);
                    return Response::Ack;
                }
                Err(e) => {
                    error!("Failed to delete profile: {}", e);
                    return Response::Error(format!("Failed to delete profile: {}", e));
                }
            }
        }
        Request::GrabDevice { device_path } => {
            // Grab a device exclusively for input interception
            let state = state.read().await;
            if let Some(device_manager) = &state.device_manager {
                let mut dm = device_manager.write().await;
                match dm.grab_device(&device_path).await {
                    Ok(_) => {
                        info!("Device {} grabbed successfully", device_path);
                        return Response::Ack;
                    }
                    Err(e) => {
                        error!("Failed to grab device {}: {}", device_path, e);
                        return Response::Error(format!("Failed to grab device: {}", e));
                    }
                }
            } else {
                return Response::Error("Device manager not initialized".to_string());
            }
        }
        Request::UngrabDevice { device_path } => {
            // Release exclusive access to a device
            let state = state.read().await;
            if let Some(device_manager) = &state.device_manager {
                let mut dm = device_manager.write().await;
                match dm.ungrab_device(&device_path).await {
                    Ok(_) => {
                        info!("Device {} ungrabbed successfully", device_path);
                        return Response::Ack;
                    }
                    Err(e) => {
                        error!("Failed to ungrab device {}: {}", device_path, e);
                        return Response::Error(format!("Failed to ungrab device: {}", e));
                    }
                }
            } else {
                return Response::Error("Device manager not initialized".to_string());
            }
        }
    }
}


/// Get the GID for a group name
#[cfg(target_os = "linux")]
fn get_group_gid(group_name: &str) -> Option<u32> {
    // Simplified implementation for now
    // In a real implementation, this would use libc or nix to resolve group names
    match group_name {
        "root" => Some(0),
        "input" => Some(1001), // Common GID for input group
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DaemonState;
    use razermapper_common::{DeviceInfo, MacroEntry, KeyCombo, Action};
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    // Helper function to create a test injector or skip the test if permissions are insufficient
    fn create_test_injector() -> Arc<injector::UinputInjector> {
        match injector::UinputInjector::new() {
            Ok(injector) => Arc::new(injector),
            Err(_) => {
                // Skip test if we don't have permission to create injector
                panic!("Test requires root access to create UinputInjector. Run with sudo or set CAP_SYS_ADMIN capability.");
            }
        }
    }

    // Helper function to create a test ConfigManager with temporary paths
    async fn create_test_config_manager() -> Arc<config::ConfigManager> {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");
        let macros_path = temp_dir.path().join("macros.yaml");
        let cache_path = temp_dir.path().join("macros.bin");
        let profiles_dir = temp_dir.path().join("profiles");

        let manager = config::ConfigManager {
            config_path,
            macros_path,
            cache_path,
            profiles_dir,
            config: config::DaemonConfig::default(),
            macros: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            profiles: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        };

        Arc::new(manager)
    }

    #[tokio::test]
    async fn test_ipc_server_creation() {
        let temp_dir = TempDir::new().unwrap();
        let socket_path = temp_dir.path().join("test.sock");

        let server = IpcServer::new(&socket_path).unwrap();
        assert_eq!(server.socket_path, socket_path.to_string_lossy());
    }

    #[tokio::test]
    async fn test_request_handling() {
        let state = Arc::new(RwLock::new(DaemonState::new()));
        let macro_engine = Arc::new(macro_engine::MacroEngine::new());
        let injector = create_test_injector();
        let config_manager = create_test_config_manager().await;
        let security_manager = Arc::new(RwLock::new(security::SecurityManager::new(false)));

        // Test GetDevices request
        let response = handle_request(Request::GetDevices, Arc::clone(&state), Arc::clone(&macro_engine), Arc::clone(&injector), Arc::clone(&config_manager), Arc::clone(&security_manager)).await;
        assert!(matches!(response, Response::Devices(_)));

        // Test GetStatus request
        let response = handle_request(Request::GetStatus, Arc::clone(&state), Arc::clone(&macro_engine), Arc::clone(&injector), Arc::clone(&config_manager), Arc::clone(&security_manager)).await;
        match response {
            Response::Status { version, .. } => assert_eq!(version, "0.1.0"),
            _ => panic!("Expected Status response"),
        }

        // Test SetMacro request with non-existent device
        let test_macro = MacroEntry {
            name: "test".to_string(),
            trigger: KeyCombo {
                keys: vec![30],
                modifiers: vec![],
            },
            actions: vec![Action::KeyPress(30)],
            device_id: None,
            enabled: true,
        };

        let response = handle_request(
            Request::SetMacro {
                device_path: "/nonexistent".to_string(),
                macro_entry: test_macro,
            },
            Arc::clone(&state),
            Arc::clone(&macro_engine),
            Arc::clone(&injector),
            Arc::clone(&config_manager),
            Arc::clone(&security_manager)
        ).await;

        match response {
            Response::Error(msg) => assert!(msg.contains("not found")),
            _ => panic!("Expected Error response"),
        }
    }

    #[tokio::test]
    async fn test_macro_addition() {
        // Create test state
        let state = Arc::new(RwLock::new(DaemonState::new()));
        let macro_engine = Arc::new(macro_engine::MacroEngine::new());
        let injector = create_test_injector();
        let config_manager = create_test_config_manager().await;
        let security_manager = Arc::new(RwLock::new(security::SecurityManager::new(false)));

        // Add a device first
        {
            let state = state.write().await;
            state.devices.lock().unwrap().push(DeviceInfo {
                name: "Test Device".to_string(),
                path: PathBuf::from("/dev/input/test"),
                vendor_id: 0x1234,
                product_id: 0x5678,
                phys: "test-phys".to_string(),
            });
        }

        // Now add a macro
        let test_macro = MacroEntry {
            name: "test".to_string(),
            trigger: KeyCombo {
                keys: vec![30],
                modifiers: vec![],
            },
            actions: vec![Action::KeyPress(30)],
            device_id: None,
            enabled: true,
        };

        let response = handle_request(
            Request::SetMacro {
                device_path: "/dev/input/test".to_string(),
                macro_entry: test_macro.clone(),
            },
            Arc::clone(&state),
            Arc::clone(&macro_engine),
            Arc::clone(&injector),
            Arc::clone(&config_manager),
            Arc::clone(&security_manager)
        ).await;

        assert!(matches!(response, Response::Ack));

        // Verify the macro was added
        let state = state.read().await;
        assert_eq!(state.macros.lock().unwrap().len(), 1);
        let macros = state.macros.lock().unwrap();
        let first_macro = macros.values().next().unwrap();
        assert_eq!(first_macro.name, test_macro.name);
    }
}
