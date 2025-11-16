use razermapper_common::{tracing, DeviceInfo};
use std::collections::HashMap;
use std::path::PathBuf;
use std::fs;
use std::os::unix::io::{AsRawFd, RawFd};
use tokio::sync::mpsc;
use tracing::{info, warn, error, debug};
use evdev::{Device as EvdevDevice, InputEventKind};

// EVIOCGRAB ioctl number for exclusive device access
const EVIOCGRAB: u64 = 0x40044590;

/// Information about a grabbed device
pub struct GrabbedDevice {
    pub info: DeviceInfo,
    pub evdev: EvdevDevice,
    pub fd: RawFd,
    pub grabbed: bool,
}

/// Manages the discovery and monitoring of input devices
pub struct DeviceManager {
    devices: HashMap<String, DeviceInfo>,
    grabbed_devices: HashMap<String, GrabbedDevice>,
    event_sender: mpsc::Sender<(String, u16, bool)>,
    event_receiver: Option<mpsc::Receiver<(String, u16, bool)>>,
}

impl DeviceManager {
    /// Create a new device manager
    pub fn new() -> Self {
        let (event_sender, event_receiver) = mpsc::channel(1000);
        Self {
            devices: HashMap::new(),
            grabbed_devices: HashMap::new(),
            event_sender,
            event_receiver: Some(event_receiver),
        }
    }

    /// Start device discovery and monitoring
    pub async fn start_discovery(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Starting device discovery");

        // Get list of devices
        let discovered_devices = self.scan_devices().await?;

        // Add devices to our collection
        for device in discovered_devices {
            info!("Found device: {} at {}", device.name, device.path.display());
            self.devices.insert(device.path.to_string_lossy().to_string(), device);
        }

        info!("Discovered {} input devices", self.devices.len());
        Ok(())
    }

    /// Get all discovered devices
    pub fn get_devices(&self) -> Vec<DeviceInfo> {
        self.devices.values().cloned().collect()
    }

    /// Get a specific device by path
    pub fn get_device(&self, path: &str) -> Option<DeviceInfo> {
        self.devices.get(path).cloned()
    }

    /// Get event receiver for new device events
    pub fn get_event_receiver(&mut self) -> mpsc::Receiver<(String, u16, bool)> {
        self.event_receiver.take().expect("Event receiver already taken")
    }

    /// Grab a device exclusively (EVIOCGRAB) for input interception
    pub async fn grab_device(&mut self, device_path: &str) -> Result<(), Box<dyn std::error::Error>> {
        if self.grabbed_devices.contains_key(device_path) {
            info!("Device {} already grabbed", device_path);
            return Ok(());
        }

        let device_info = self.devices.get(device_path)
            .ok_or_else(|| format!("Device not found: {}", device_path))?
            .clone();

        info!("Grabbing device: {} ({})", device_info.name, device_path);

        // Open the evdev device
        let evdev = EvdevDevice::open(device_path)
            .map_err(|e| format!("Failed to open device {}: {}", device_path, e))?;

        let fd = evdev.as_raw_fd();

        // Grab the device exclusively with EVIOCGRAB
        let result = unsafe {
            libc::ioctl(fd, EVIOCGRAB, 1 as libc::c_int)
        };

        if result < 0 {
            let err = std::io::Error::last_os_error();
            error!("Failed to grab device {}: {}", device_path, err);
            return Err(format!("EVIOCGRAB failed: {}", err).into());
        }

        info!("Successfully grabbed device {} (fd={})", device_path, fd);

        // Store the grabbed device
        self.grabbed_devices.insert(device_path.to_string(), GrabbedDevice {
            info: device_info,
            evdev,
            fd,
            grabbed: true,
        });

        // Start event reading loop for this device
        self.start_event_reader(device_path.to_string()).await?;

        Ok(())
    }

    /// Ungrab a device (release exclusive access)
    pub async fn ungrab_device(&mut self, device_path: &str) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(grabbed) = self.grabbed_devices.remove(device_path) {
            info!("Ungrabbing device: {}", device_path);

            // Release the grab
            let result = unsafe {
                libc::ioctl(grabbed.fd, EVIOCGRAB, 0 as libc::c_int)
            };

            if result < 0 {
                warn!("Failed to ungrab device {}: {}", device_path, std::io::Error::last_os_error());
            } else {
                info!("Successfully ungrabbed device {}", device_path);
            }
        }

        Ok(())
    }

    /// Start reading events from a grabbed device
    async fn start_event_reader(&self, device_path: String) -> Result<(), Box<dyn std::error::Error>> {
        let sender = self.event_sender.clone();

        // Clone the path for the async task
        let path = device_path.clone();

        // Spawn a blocking task since evdev uses synchronous I/O
        tokio::task::spawn_blocking(move || {
            info!("Starting event reader for {}", path);

            // Re-open the device in the blocking context
            let mut device = match EvdevDevice::open(&path) {
                Ok(d) => d,
                Err(e) => {
                    error!("Failed to open device {} for event reading: {}", path, e);
                    return;
                }
            };

            // Create a runtime handle for sending events
            let rt = tokio::runtime::Handle::current();

            loop {
                // Fetch events synchronously (this blocks)
                match device.fetch_events() {
                    Ok(events) => {
                        for event in events {
                            // Only process key events
                            if let InputEventKind::Key(key) = event.kind() {
                                let key_code = key.0;
                                let pressed = event.value() == 1; // 1 = pressed, 0 = released

                                debug!("Event from {}: key={}, pressed={}", path, key_code, pressed);

                                // Send event to macro engine using blocking send
                                let sender_clone = sender.clone();
                                let path_clone = path.clone();
                                if let Err(e) = rt.block_on(sender_clone.send((path_clone, key_code, pressed))) {
                                    error!("Failed to send event: {}", e);
                                    return;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error reading event from {}: {}", path, e);
                        break;
                    }
                }
            }

            info!("Event reader stopped for {}", path);
        });

        Ok(())
    }

    /// Scan for input devices
    async fn scan_devices(&self) -> Result<Vec<DeviceInfo>, Box<dyn std::error::Error>> {
        let mut devices: Vec<DeviceInfo> = Vec::new();

        // First, try to discover Razer devices through openrazer sysfs
        if let Ok(razer_devices) = self.scan_razer_sysfs().await {
            for device in razer_devices {
                info!("Found Razer device via sysfs: {}", device.name);
                devices.push(device);
            }
        }

        // Then, scan all /dev/input/event* devices
        for entry in fs::read_dir("/dev/input")? {
            let entry = entry?;
            let path = entry.path();

            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if name.starts_with("event") {
                    if let Ok(device_info) = self.get_device_info(&path).await {
                        // Check if we already have this device (from Razer scan)
                        let already_exists = devices.iter()
                            .any(|d| d.path == device_info.path);

                        if !already_exists {
                            devices.push(device_info);
                        }
                    }
                }
            }
        }

        Ok(devices)
    }

    /// Get device information by opening it with evdev
    async fn get_device_info(&self, path: &PathBuf) -> Result<DeviceInfo, Box<dyn std::error::Error>> {
        let device = EvdevDevice::open(path)
            .map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;

        let name = device.name().unwrap_or("Unknown Device").to_string();

        // Get input_id from evdev
        let input_id = device.input_id();

        let phys = device.physical_path()
            .unwrap_or("unknown")
            .to_string();

        Ok(DeviceInfo {
            name,
            path: path.clone(),
            vendor_id: input_id.vendor(),
            product_id: input_id.product(),
            phys,
        })
    }

    /// Scan for Razer devices via sysfs (direct integration with OpenRazer)
    async fn scan_razer_sysfs(&self) -> Result<Vec<DeviceInfo>, Box<dyn std::error::Error>> {
        let mut devices: Vec<DeviceInfo> = Vec::new();

        // OpenRazer kernel module exposes devices at these paths
        let driver_paths = vec![
            "/sys/bus/hid/drivers/razerkbd",
            "/sys/bus/hid/drivers/razermouse",
            "/sys/bus/hid/drivers/razerchroma",
        ];

        for driver_path in driver_paths {
            if let Ok(entries) = fs::read_dir(driver_path) {
                for entry in entries {
                    let entry = entry?;
                    let path = entry.path();

                    // Device directories have format: XXXX:1532:YYYY.ZZZZ
                    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                        if name.contains(":1532:") {
                            // This is a Razer device (VID 1532)
                            if let Ok(device_info) = self.parse_razer_sysfs(&path).await {
                                info!("Found Razer device in sysfs: {}", device_info.name);
                                devices.push(device_info);
                            }
                        }
                    }
                }
            }
        }

        Ok(devices)
    }

    /// Parse Razer device information from sysfs
    async fn parse_razer_sysfs(&self, sysfs_path: &PathBuf) -> Result<DeviceInfo, Box<dyn std::error::Error>> {
        // Extract device type from sysfs
        let device_type_path = sysfs_path.join("device_type");
        let device_type = if device_type_path.exists() {
            fs::read_to_string(&device_type_path)
                .unwrap_or_else(|_| "Unknown Razer Device".to_string())
                .trim()
                .to_string()
        } else {
            "Razer Device".to_string()
        };

        // Parse VID/PID from directory name (format: XXXX:1532:YYYY.ZZZZ)
        let dir_name = sysfs_path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let parts: Vec<&str> = dir_name.split(':').collect();
        let (vendor_id, product_id) = if parts.len() >= 3 {
            let vid = u16::from_str_radix("1532", 16).unwrap_or(0x1532);
            let pid_part = parts[2].split('.').next().unwrap_or("0000");
            let pid = u16::from_str_radix(pid_part, 16).unwrap_or(0);
            (vid, pid)
        } else {
            (0x1532, 0x0000)
        };

        // Find the corresponding /dev/input/event* device
        let event_path = self.find_event_device_for_sysfs(sysfs_path).await
            .unwrap_or_else(|| PathBuf::from("/dev/input/event0"));

        Ok(DeviceInfo {
            name: device_type,
            path: event_path,
            vendor_id,
            product_id,
            phys: sysfs_path.to_string_lossy().to_string(),
        })
    }

    /// Find the /dev/input/event* path for a sysfs device
    async fn find_event_device_for_sysfs(&self, sysfs_path: &PathBuf) -> Option<PathBuf> {
        // Look for input subdirectory
        let input_dir = sysfs_path.join("input");
        if !input_dir.exists() {
            return None;
        }

        // Find input* directory
        if let Ok(entries) = fs::read_dir(&input_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if name.starts_with("input") {
                        // Look for event* in this directory
                        if let Ok(event_entries) = fs::read_dir(&path) {
                            for event_entry in event_entries.flatten() {
                                let event_path = event_entry.path();
                                if let Some(event_name) = event_path.file_name().and_then(|s| s.to_str()) {
                                    if event_name.starts_with("event") {
                                        // Extract event number
                                        let event_num = event_name.replace("event", "");
                                        return Some(PathBuf::from(format!("/dev/input/event{}", event_num)));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Create a fallback device for testing
    fn create_fallback_device(&self, device_type: &str) -> DeviceInfo {
        DeviceInfo {
            name: format!("Fallback {}", device_type),
            path: PathBuf::from(format!("/dev/input/event{}", if device_type == "keyboard" { "0" } else { "1" })),
            vendor_id: 0x1532,
            product_id: 0x0220,
            phys: "fallback-device".to_string(),
        }
    }

    /// Shutdown the device manager
    pub async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Shutting down device manager");

        // Ungrab all devices
        let device_paths: Vec<String> = self.grabbed_devices.keys().cloned().collect();
        for path in device_paths {
            if let Err(e) = self.ungrab_device(&path).await {
                warn!("Error ungrabbing device {}: {}", path, e);
            }
        }

        info!("Device manager shutdown complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_device_manager_creation() {
        let manager = DeviceManager::new();
        assert!(manager.devices.is_empty());
        assert!(manager.grabbed_devices.is_empty());
    }

    #[tokio::test]
    async fn test_device_discovery() {
        let mut manager = DeviceManager::new();

        // This test requires /dev/input access
        // In a non-privileged test environment, it may fail
        let result = manager.start_discovery().await;

        // Just check that it doesn't panic
        // In a real test environment with devices, check for devices
        if result.is_ok() {
            // We should have found some devices (at least virtual ones)
            // On a system with no input devices, this may be empty
            println!("Found {} devices", manager.get_devices().len());
        }
    }
}
