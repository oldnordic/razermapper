use iced::{
    widget::{button, column, container, row, text, text_input, scrollable, horizontal_rule, Column, Row},
    Element, Length, Subscription, Theme, Application, Command,
};
use razermapper_common::{DeviceInfo, MacroEntry};
use std::path::PathBuf;
use std::collections::{VecDeque, HashMap, HashSet};
use std::time::{Duration, Instant};

pub struct State {
    pub devices: Vec<DeviceInfo>,
    pub macros: Vec<MacroEntry>,
    pub selected_device: Option<usize>,
    pub status: String,
    pub status_history: VecDeque<String>,
    pub loading: bool,
    pub recording: bool,
    pub recording_macro_name: Option<String>,
    pub daemon_connected: bool,
    pub new_macro_name: String,
    pub socket_path: PathBuf,
    pub recently_updated_macros: HashMap<String, Instant>,
    pub grabbed_devices: HashSet<String>,
    pub profile_name: String,
}

impl Default for State {
    fn default() -> Self {
        let socket_path = if cfg!(target_os = "linux") {
            PathBuf::from("/run/razermapper/razermapper.sock")
        } else if cfg!(target_os = "macos") {
            PathBuf::from("/tmp/razermapper.sock")
        } else {
            std::env::temp_dir().join("razermapper.sock")
        };
        State {
            devices: Vec::new(),
            macros: Vec::new(),
            selected_device: None,
            status: "Initializing...".to_string(),
            status_history: VecDeque::with_capacity(5),
            loading: false,
            recording: false,
            recording_macro_name: None,
            daemon_connected: false,
            new_macro_name: String::new(),
            socket_path,
            recently_updated_macros: HashMap::new(),
            grabbed_devices: HashSet::new(),
            profile_name: "default".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    LoadDevices,
    DevicesLoaded(Result<Vec<DeviceInfo>, String>),
    LoadMacros,
    MacrosLoaded(Result<Vec<MacroEntry>, String>),
    PlayMacro(String),
    StartRecording,
    StopRecording,
    RecordingStarted(Result<String, String>),
    RecordingStopped(Result<MacroEntry, String>),
    DeleteMacro(String),
    MacroDeleted(Result<String, String>),
    SaveProfile,
    ProfileSaved(Result<(String, usize), String>),
    LoadProfile,
    ProfileLoaded(Result<(String, usize), String>),
    UpdateMacroName(String),
    UpdateProfileName(String),
    CheckDaemonConnection,
    DaemonStatusChanged(bool),
    TickAnimations,
    GrabDevice(String),
    UngrabDevice(String),
    DeviceGrabbed(Result<String, String>),
    DeviceUngrabbed(Result<String, String>),
    MacroPlayed(Result<String, String>),
}

impl Application for State {
    type Message = Message;
    type Theme = Theme;
    type Executor = iced::executor::Default;
    type Flags = ();

    fn new(_flags: ()) -> (Self, Command<Message>) {
        let initial_state = State::default();
        let initial_commands = Command::batch([
            Command::perform(async { Message::CheckDaemonConnection }, |msg| msg),
            Command::perform(async { Message::LoadDevices }, |msg| msg),
        ]);
        (initial_state, initial_commands)
    }

    fn title(&self) -> String {
        String::from("Razermapper - Input Device Remapper")
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::CheckDaemonConnection => {
                let socket_path = self.socket_path.clone();
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.connect().await.is_ok()
                    },
                    Message::DaemonStatusChanged,
                )
            }
            Message::DaemonStatusChanged(connected) => {
                self.daemon_connected = connected;
                if connected {
                    self.update_status("Connected to daemon".to_string());
                } else {
                    self.update_status("Daemon not running - start razermapperd".to_string());
                }
                Command::none()
            }
            Message::LoadDevices => {
                let socket_path = self.socket_path.clone();
                self.loading = true;
                self.update_status("Loading devices...".to_string());
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.get_devices().await.map_err(|e| e.to_string())
                    },
                    Message::DevicesLoaded,
                )
            }
            Message::DevicesLoaded(Ok(devices)) => {
                self.devices = devices;
                self.loading = false;
                self.update_status(format!("Found {} devices", self.devices.len()));
                Command::perform(async { Message::LoadMacros }, |msg| msg)
            }
            Message::DevicesLoaded(Err(e)) => {
                self.loading = false;
                self.update_status(format!("Error: {}", e));
                Command::none()
            }
            Message::LoadMacros => {
                let socket_path = self.socket_path.clone();
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.list_macros().await.map_err(|e| e.to_string())
                    },
                    Message::MacrosLoaded,
                )
            }
            Message::MacrosLoaded(Ok(macros)) => {
                self.macros = macros;
                self.update_status(format!("Loaded {} macros", self.macros.len()));
                Command::none()
            }
            Message::MacrosLoaded(Err(e)) => {
                self.update_status(format!("Error loading macros: {}", e));
                Command::none()
            }
            Message::PlayMacro(macro_name) => {
                let socket_path = self.socket_path.clone();
                let name = macro_name.clone();
                self.update_status(format!("Playing macro: {}", macro_name));
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.test_macro(&name).await.map(|_| name).map_err(|e| e.to_string())
                    },
                    Message::MacroPlayed,
                )
            }
            Message::MacroPlayed(Ok(name)) => {
                self.update_status(format!("Played macro: {}", name));
                Command::none()
            }
            Message::MacroPlayed(Err(e)) => {
                self.update_status(format!("Failed to play: {}", e));
                Command::none()
            }
            Message::UpdateMacroName(name) => {
                self.new_macro_name = name;
                Command::none()
            }
            Message::UpdateProfileName(name) => {
                self.profile_name = name;
                Command::none()
            }
            Message::StartRecording => {
                if self.new_macro_name.trim().is_empty() {
                    self.update_status("Enter a macro name first".to_string());
                    return Command::none();
                }
                if self.selected_device.is_none() {
                    self.update_status("Select and grab a device first".to_string());
                    return Command::none();
                }

                let device_idx = self.selected_device.unwrap();
                let device_path = self.devices[device_idx].path.to_string_lossy().to_string();

                if !self.grabbed_devices.contains(&device_path) {
                    self.update_status("Grab the device before recording".to_string());
                    return Command::none();
                }

                let socket_path = self.socket_path.clone();
                let macro_name = self.new_macro_name.clone();
                self.recording = true;
                self.recording_macro_name = Some(macro_name.clone());
                self.update_status(format!("Recording macro: {}", macro_name));

                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.start_recording_macro(&device_path, &macro_name)
                            .await
                            .map(|_| macro_name)
                            .map_err(|e| e.to_string())
                    },
                    Message::RecordingStarted,
                )
            }
            Message::RecordingStarted(Ok(name)) => {
                self.update_status(format!("Recording: {} - Press keys now", name));
                Command::none()
            }
            Message::RecordingStarted(Err(e)) => {
                self.recording = false;
                self.recording_macro_name = None;
                self.update_status(format!("Failed to start recording: {}", e));
                Command::none()
            }
            Message::StopRecording => {
                let socket_path = self.socket_path.clone();
                self.update_status("Stopping recording...".to_string());
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.stop_recording_macro().await.map_err(|e| e.to_string())
                    },
                    Message::RecordingStopped,
                )
            }
            Message::RecordingStopped(Ok(macro_entry)) => {
                let name = macro_entry.name.clone();
                self.macros.push(macro_entry);
                self.recording = false;
                self.recording_macro_name = None;
                self.recently_updated_macros.insert(name.clone(), Instant::now());
                self.new_macro_name.clear();
                self.update_status(format!("Recorded macro: {}", name));
                Command::none()
            }
            Message::RecordingStopped(Err(e)) => {
                self.recording = false;
                self.recording_macro_name = None;
                self.update_status(format!("Recording failed: {}", e));
                Command::none()
            }
            Message::DeleteMacro(macro_name) => {
                let socket_path = self.socket_path.clone();
                let name = macro_name.clone();
                self.update_status(format!("Deleting: {}", macro_name));
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.delete_macro(&name).await.map(|_| name).map_err(|e| e.to_string())
                    },
                    Message::MacroDeleted,
                )
            }
            Message::MacroDeleted(Ok(name)) => {
                self.macros.retain(|m| m.name != name);
                self.update_status(format!("Deleted: {}", name));
                Command::none()
            }
            Message::MacroDeleted(Err(e)) => {
                self.update_status(format!("Delete failed: {}", e));
                Command::none()
            }
            Message::SaveProfile => {
                if self.profile_name.trim().is_empty() {
                    self.update_status("Enter a profile name".to_string());
                    return Command::none();
                }
                let socket_path = self.socket_path.clone();
                let name = self.profile_name.clone();
                self.update_status(format!("Saving profile: {}", name));
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.save_profile(&name).await.map_err(|e| e.to_string())
                    },
                    Message::ProfileSaved,
                )
            }
            Message::ProfileSaved(Ok((name, count))) => {
                self.update_status(format!("Saved profile '{}' with {} macros", name, count));
                Command::none()
            }
            Message::ProfileSaved(Err(e)) => {
                self.update_status(format!("Save failed: {}", e));
                Command::none()
            }
            Message::LoadProfile => {
                if self.profile_name.trim().is_empty() {
                    self.update_status("Enter a profile name to load".to_string());
                    return Command::none();
                }
                let socket_path = self.socket_path.clone();
                let name = self.profile_name.clone();
                self.update_status(format!("Loading profile: {}", name));
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.load_profile(&name).await.map_err(|e| e.to_string())
                    },
                    Message::ProfileLoaded,
                )
            }
            Message::ProfileLoaded(Ok((name, count))) => {
                self.update_status(format!("Loaded '{}' ({} macros)", name, count));
                Command::perform(async { Message::LoadMacros }, |msg| msg)
            }
            Message::ProfileLoaded(Err(e)) => {
                self.update_status(format!("Load failed: {}", e));
                Command::none()
            }
            Message::TickAnimations => {
                let now = Instant::now();
                self.recently_updated_macros.retain(|_, timestamp| {
                    now.duration_since(*timestamp) < Duration::from_secs(2)
                });
                Command::none()
            }
            Message::GrabDevice(device_path) => {
                let socket_path = self.socket_path.clone();
                let path_clone = device_path.clone();
                self.update_status("Grabbing device...".to_string());
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.grab_device(&path_clone).await.map(|_| path_clone).map_err(|e| e.to_string())
                    },
                    Message::DeviceGrabbed,
                )
            }
            Message::UngrabDevice(device_path) => {
                let socket_path = self.socket_path.clone();
                let path_clone = device_path.clone();
                self.update_status("Releasing device...".to_string());
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.ungrab_device(&path_clone).await.map(|_| path_clone).map_err(|e| e.to_string())
                    },
                    Message::DeviceUngrabbed,
                )
            }
            Message::DeviceGrabbed(Ok(device_path)) => {
                self.grabbed_devices.insert(device_path.clone());
                if let Some(idx) = self.devices.iter().position(|d| d.path.to_string_lossy() == device_path) {
                    self.selected_device = Some(idx);
                }
                self.update_status("Device grabbed - ready for recording".to_string());
                Command::none()
            }
            Message::DeviceGrabbed(Err(e)) => {
                self.update_status(format!("Grab failed: {}", e));
                Command::none()
            }
            Message::DeviceUngrabbed(Ok(device_path)) => {
                self.grabbed_devices.remove(&device_path);
                self.update_status("Device released".to_string());
                Command::none()
            }
            Message::DeviceUngrabbed(Err(e)) => {
                self.update_status(format!("Release failed: {}", e));
                Command::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        // Header section
        let header = row![
            text("RAZERMAPPER").size(28),
            text(" | Input Device Remapper").size(16),
        ]
        .spacing(5)
        .align_items(iced::Alignment::End);

        // Connection status indicator
        let connection_indicator = if self.daemon_connected {
            text("● CONNECTED").size(14)
        } else {
            text("○ DISCONNECTED").size(14)
        };

        // Status bar
        let status_bar = row![
            connection_indicator,
            text(" | ").size(14),
            text(&self.status).size(14),
        ]
        .spacing(5);

        // Toolbar buttons
        let toolbar = row![
            button("Refresh Connection")
                .on_press(Message::CheckDaemonConnection)
                .style(iced::theme::Button::Secondary),
            button("Reload Devices")
                .on_press(Message::LoadDevices)
                .style(iced::theme::Button::Secondary),
        ]
        .spacing(10);

        // Devices section
        let devices_section = self.view_devices_section();

        // Macro recording section
        let recording_section = self.view_recording_section();

        // Macros list section
        let macros_section = self.view_macros_section();

        // Profile management section
        let profile_section = self.view_profile_section();

        // Main layout
        let content = column![
            header,
            horizontal_rule(1),
            status_bar,
            toolbar,
            horizontal_rule(1),
            text("DEVICES").size(18),
            devices_section,
            horizontal_rule(1),
            text("MACRO RECORDING").size(18),
            recording_section,
            horizontal_rule(1),
            text("MACROS").size(18),
            macros_section,
            horizontal_rule(1),
            text("PROFILES").size(18),
            profile_section,
        ]
        .spacing(10)
        .padding(20);

        container(scrollable(content))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x()
            .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_millis(100)).map(|_| Message::TickAnimations)
    }
}

impl State {
    fn update_status(&mut self, status: String) {
        self.status = status.clone();
        self.status_history.push_back(status);
        if self.status_history.len() > 5 {
            self.status_history.pop_front();
        }
    }

    fn view_devices_section(&self) -> Element<'_, Message> {
        if self.devices.is_empty() {
            return text("No devices found. Click 'Reload Devices' to scan.").size(12).into();
        }

        let mut device_list: Column<Message> = column![].spacing(8);

        for (idx, device) in self.devices.iter().enumerate() {
            let device_path = device.path.to_string_lossy().to_string();
            let is_grabbed = self.grabbed_devices.contains(&device_path);
            let is_selected = self.selected_device == Some(idx);

            let device_name = if is_selected {
                text(format!("► {}", device.name)).size(14)
            } else {
                text(&device.name).size(14)
            };

            let device_info = text(format!(
                "VID:{:04X} PID:{:04X} | {}",
                device.vendor_id, device.product_id, device_path
            ))
            .size(10);

            let status_text = if is_grabbed {
                text("GRABBED").size(10)
            } else {
                text("").size(10)
            };

            let grab_button = if is_grabbed {
                button("Release")
                    .on_press(Message::UngrabDevice(device_path.clone()))
                    .style(iced::theme::Button::Destructive)
            } else {
                button("Grab")
                    .on_press(Message::GrabDevice(device_path.clone()))
                    .style(iced::theme::Button::Primary)
            };

            let device_row: Row<Message> = row![
                column![device_name, device_info].spacing(2),
                iced::widget::Space::with_width(Length::Fill),
                status_text,
                grab_button,
            ]
            .spacing(10)
            .align_items(iced::Alignment::Center);

            device_list = device_list.push(device_row);
        }

        container(device_list)
            .padding(10)
            .width(Length::Fill)
            .style(iced::theme::Container::Box)
            .into()
    }

    fn view_recording_section(&self) -> Element<'_, Message> {
        let macro_name_input = text_input("Enter macro name...", &self.new_macro_name)
            .on_input(Message::UpdateMacroName)
            .padding(10)
            .size(14);

        let record_button = if self.recording {
            button("STOP RECORDING")
                .on_press(Message::StopRecording)
                .style(iced::theme::Button::Destructive)
        } else {
            button("RECORD MACRO")
                .on_press(Message::StartRecording)
                .style(iced::theme::Button::Primary)
        };

        let recording_status = if self.recording {
            text(format!(
                "Recording: {} - Press keys on the grabbed device",
                self.recording_macro_name.as_deref().unwrap_or("")
            ))
            .size(12)
        } else {
            text("1. Grab a device  2. Enter macro name  3. Click Record").size(12)
        };

        let content = column![
            row![
                macro_name_input,
                record_button,
            ]
            .spacing(10)
            .align_items(iced::Alignment::Center),
            recording_status,
        ]
        .spacing(10);

        container(content)
            .padding(10)
            .width(Length::Fill)
            .style(iced::theme::Container::Box)
            .into()
    }

    fn view_macros_section(&self) -> Element<'_, Message> {
        let header_row: Row<Message> = row![
            text(format!("Total: {}", self.macros.len())).size(12),
        ]
        .spacing(10);

        let mut macro_list: Column<Message> = column![header_row].spacing(8);

        if self.macros.is_empty() {
            macro_list = macro_list.push(text("No macros configured yet.").size(12));
        } else {
            for macro_entry in &self.macros {
                let is_recent = self.recently_updated_macros.contains_key(&macro_entry.name);
                let macro_name = macro_entry.name.clone();
                let action_count = macro_entry.actions.len();
                let trigger_keys = macro_entry.trigger.keys.len();

                let name_text = if is_recent {
                    text(format!("★ {}", macro_entry.name)).size(14)
                } else {
                    text(&macro_entry.name).size(14)
                };

                let info_text = text(format!(
                    "{} actions | {} trigger keys | {}",
                    action_count,
                    trigger_keys,
                    if macro_entry.enabled { "enabled" } else { "disabled" }
                ))
                .size(10);

                let play_button = button("Play")
                    .on_press(Message::PlayMacro(macro_name.clone()))
                    .style(iced::theme::Button::Secondary);

                let delete_button = button("Delete")
                    .on_press(Message::DeleteMacro(macro_name))
                    .style(iced::theme::Button::Destructive);

                let macro_row: Row<Message> = row![
                    column![name_text, info_text].spacing(2),
                    iced::widget::Space::with_width(Length::Fill),
                    play_button,
                    delete_button,
                ]
                .spacing(8)
                .align_items(iced::Alignment::Center);

                macro_list = macro_list.push(macro_row);
            }
        }

        container(scrollable(macro_list).height(200))
            .padding(10)
            .width(Length::Fill)
            .style(iced::theme::Container::Box)
            .into()
    }

    fn view_profile_section(&self) -> Element<'_, Message> {
        let profile_input = text_input("Profile name...", &self.profile_name)
            .on_input(Message::UpdateProfileName)
            .padding(10)
            .size(14);

        let save_button = button("Save Profile")
            .on_press(Message::SaveProfile)
            .style(iced::theme::Button::Primary);

        let load_button = button("Load Profile")
            .on_press(Message::LoadProfile)
            .style(iced::theme::Button::Secondary);

        let content = row![
            profile_input,
            save_button,
            load_button,
        ]
        .spacing(10)
        .align_items(iced::Alignment::Center);

        container(content)
            .padding(10)
            .width(Length::Fill)
            .style(iced::theme::Container::Box)
            .into()
    }
}
