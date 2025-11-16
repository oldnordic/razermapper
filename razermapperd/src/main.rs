//! Razermapper Daemon - Main Entry Point
//!
//! This is the privileged system daemon responsible for:
//! - Device discovery and management
//! - Macro recording and playback
//! - IPC communication with the GUI client
//! - Security management and privilege dropping

use razermapper_common::tracing;
use razermapperd::{DaemonState, config, device, macro_engine, injector, ipc, security};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, error, warn};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check for test mode first
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "--test-security" {
        return security::test_security_functionality().await;
    }

// Main daemon implementation
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    info!("Starting Razermapper Daemon v0.1.0");

    // Check if we're running as root (required for privileged operations)
    if !security::SecurityManager::is_root() {
        error!("Razermapper daemon must be started as root for device access");
        return Err("Insufficient privileges".into());
    }

    // Determine socket path
    let socket_path = determine_socket_path()?;
    info!("Using socket path: {}", socket_path);

    // Initialize security manager with token authentication based on feature flag
    let token_auth_enabled = cfg!(feature = "token-auth");
    let security_manager = Arc::new(RwLock::new(security::SecurityManager::new(token_auth_enabled)));
    info!("Token authentication {}", if token_auth_enabled { "enabled" } else { "disabled" });

    // Create shared state
    let state = Arc::new(RwLock::new(DaemonState::new()));

    // Initialize components
    let config_manager = Arc::new(config::ConfigManager::new().await?);
    let injector = injector::UinputInjector::new()?;

    // Initialize injector with full privileges before dropping them
    {
        injector.initialize().await.map_err(|e| -> Box<dyn std::error::Error> { e })?;
        info!("Uinput injector initialized");
    }

    // Wrap injector in Arc<RwLock<dyn Injector>> for MacroEngine
    let injector_for_macro: Arc<RwLock<dyn injector::Injector + Send + Sync>> =
        Arc::new(RwLock::new(injector));

    // Clone Arc for IPC server (it can downcast or use trait methods)
    let injector_for_ipc = Arc::clone(&injector_for_macro);

    // Create and initialize device manager
    let mut device_manager = device::DeviceManager::new();
    if let Err(e) = device_manager.start_discovery().await {
        error!("Device discovery failed: {}", e);
    } else {
        info!("Device discovery successful");

        // Update devices in shared state
        let discovered_devices = device_manager.get_devices();
        {
            let state = state.write().await;
            *state.devices.lock().unwrap() = discovered_devices;
        }

        // Start device event processing loop
        let event_receiver = device_manager.get_event_receiver();

        // Wrap device_manager in Arc<RwLock<>> for sharing with IPC
        let device_manager = Arc::new(RwLock::new(device_manager));
        {
            let mut state = state.write().await;
            state.device_manager = Some(Arc::clone(&device_manager));
        }

        let state_clone = Arc::clone(&state);

        let state_clone2 = Arc::clone(&state_clone);
        tokio::spawn(async move {
            let mut event_receiver = event_receiver;
            loop {
                if let Some((device_path, key_code, pressed)) = event_receiver.recv().await {
                    // Forward event to macro engine for processing
                    let state = state_clone2.read().await;
                    if let Some(macro_engine) = &state.macro_engine {
                        if let Err(e) = macro_engine.process_input_event(
                            key_code,
                            pressed,
                            &device_path
                        ).await {
                            error!("Error processing input event: {}", e);
                        }
                    }
                }
            }
        });
    }

    // Initialize macro engine with injector
    let macro_engine = Arc::new(macro_engine::MacroEngine::with_injector(Arc::clone(&injector_for_macro)));
    {
        let mut state = state.write().await;
        state.macro_engine = Some(Arc::clone(&macro_engine));
    }

    // Load configuration
    config_manager.load_config_mut().await?;

    // Load macros from the default profile
    if let Some(default_profile) = config_manager.get_profile("default").await {
        for (macro_name, macro_entry) in &default_profile.macros {
            if let Err(e) = macro_engine.add_macro(macro_entry.clone()).await {
                error!("Failed to add macro '{}' from profile: {}", macro_name, e);
            }
        }
    }

    // AFTER completing all privileged initialization (uinput, device discovery, etc.)
    // Drop privileges to minimize attack surface
    {
        let mut security = security_manager.write().await;
        if let Err(e) = security.drop_privileges() {
            error!("Failed to drop privileges: {}", e);
        } else {
            info!("Successfully dropped privileges after initialization");
        }
    }

    // Start IPC server
    let mut ipc_server = ipc::IpcServer::new(&socket_path)?;
    let state_for_shutdown = Arc::clone(&state);
    ipc_server.start(
        state,
        macro_engine,
        injector_for_ipc,
        config_manager,
        security_manager
    ).await?;
    info!("IPC server started successfully");

    // Set up signal handlers for graceful shutdown
    let mut signals = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    // Wait for shutdown signal
    tokio::select! {
        _ = signals.recv() => {
            info!("Received SIGTERM, shutting down gracefully");
        }
        _ = interrupt.recv() => {
            info!("Received SIGINT, shutting down gracefully");
        }
    }

    // Cleanup
    info!("Starting cleanup...");

    // Shutdown device manager first (ungrab all devices)
    {
        let state = state_for_shutdown.read().await;
        if let Some(device_manager) = &state.device_manager {
            let mut dm = device_manager.write().await;
            if let Err(e) = dm.shutdown().await {
                error!("Error during device manager shutdown: {}", e);
            }
        }
    }

    ipc_server.shutdown().await?;
    info!("Razermapper Daemon shutdown complete");
    Ok(())
}

/// Determine the appropriate socket path based on the platform
fn determine_socket_path() -> Result<String, Box<dyn std::error::Error>> {
    // For system daemon running as root, use RuntimeDirectory from systemd
    // This is created by RuntimeDirectory=razermapper in the service file
    let path = "/run/razermapper/razermapper.sock".to_string();
    info!("Using system-wide socket location: {}", path);
    Ok(path)
}
