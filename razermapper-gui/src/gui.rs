use iced::{
    widget::{
        button, column, container, row, text, text_input, scrollable,
        horizontal_rule, vertical_rule, Column, Space,
    },
    Element, Length, Subscription, Theme, Application, Command,
    Alignment,
};
use razermapper_common::{DeviceInfo, MacroEntry};
use std::path::PathBuf;
use std::collections::{VecDeque, HashMap, HashSet};
use std::time::{Duration, Instant};

// Razer brand colors (for future custom theming)
// const RAZER_GREEN: Color = Color::from_rgb(0.267, 0.839, 0.173); // #44D62C
// const RAZER_GREEN_DIM: Color = Color::from_rgb(0.176, 0.561, 0.118); // #2D8F1E
// const BG_DEEP: Color = Color::from_rgb(0.051, 0.051, 0.051); // #0D0D0D
// const BG_SURFACE: Color = Color::from_rgb(0.102, 0.102, 0.102); // #1A1A1A
// const BG_ELEVATED: Color = Color::from_rgb(0.141, 0.141, 0.141); // #242424
// const TEXT_PRIMARY: Color = Color::WHITE;
// const TEXT_SECONDARY: Color = Color::from_rgb(0.702, 0.702, 0.702); // #B3B3B3
// const TEXT_MUTED: Color = Color::from_rgb(0.400, 0.400, 0.400); // #666666
// const DANGER_RED: Color = Color::from_rgb(1.0, 0.231, 0.188); // #FF3B30
// const WARNING_YELLOW: Color = Color::from_rgb(1.0, 0.722, 0.0); // #FFB800

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Devices,
    Macros,
    Profiles,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub message: String,
    pub is_error: bool,
    pub timestamp: Instant,
}

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
    pub active_tab: Tab,
    pub notifications: VecDeque<Notification>,
    pub recording_pulse: bool,
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
            status_history: VecDeque::with_capacity(10),
            loading: false,
            recording: false,
            recording_macro_name: None,
            daemon_connected: false,
            new_macro_name: String::new(),
            socket_path,
            recently_updated_macros: HashMap::new(),
            grabbed_devices: HashSet::new(),
            profile_name: "default".to_string(),
            active_tab: Tab::Devices,
            notifications: VecDeque::with_capacity(5),
            recording_pulse: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    // Navigation
    SwitchTab(Tab),

    // Device Management
    LoadDevices,
    DevicesLoaded(Result<Vec<DeviceInfo>, String>),
    GrabDevice(String),
    UngrabDevice(String),
    DeviceGrabbed(Result<String, String>),
    DeviceUngrabbed(Result<String, String>),
    SelectDevice(usize),

    // Macro Recording
    UpdateMacroName(String),
    StartRecording,
    StopRecording,
    RecordingStarted(Result<String, String>),
    RecordingStopped(Result<MacroEntry, String>),

    // Macro Management
    LoadMacros,
    MacrosLoaded(Result<Vec<MacroEntry>, String>),
    PlayMacro(String),
    MacroPlayed(Result<String, String>),
    DeleteMacro(String),
    MacroDeleted(Result<String, String>),

    // Profile Management
    UpdateProfileName(String),
    SaveProfile,
    ProfileSaved(Result<(String, usize), String>),
    LoadProfile,
    ProfileLoaded(Result<(String, usize), String>),

    // Status
    CheckDaemonConnection,
    DaemonStatusChanged(bool),

    // UI
    TickAnimations,
}

// Reserved for future use
#[allow(dead_code)]
pub enum _FutureMessage {
    DismissNotification,
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
        String::from("Razermapper")
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::SwitchTab(tab) => {
                self.active_tab = tab;
                Command::none()
            }
            Message::SelectDevice(idx) => {
                self.selected_device = Some(idx);
                Command::none()
            }
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
                    self.add_notification("Connected to daemon", false);
                } else {
                    self.add_notification("Daemon not running - start razermapperd", true);
                }
                Command::none()
            }
            Message::LoadDevices => {
                let socket_path = self.socket_path.clone();
                self.loading = true;
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.get_devices().await.map_err(|e| e.to_string())
                    },
                    Message::DevicesLoaded,
                )
            }
            Message::DevicesLoaded(Ok(devices)) => {
                let count = devices.len();
                self.devices = devices;
                self.loading = false;
                self.add_notification(&format!("Found {} devices", count), false);
                Command::perform(async { Message::LoadMacros }, |msg| msg)
            }
            Message::DevicesLoaded(Err(e)) => {
                self.loading = false;
                self.add_notification(&format!("Error: {}", e), true);
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
                let count = macros.len();
                self.macros = macros;
                self.add_notification(&format!("Loaded {} macros", count), false);
                Command::none()
            }
            Message::MacrosLoaded(Err(e)) => {
                self.add_notification(&format!("Error loading macros: {}", e), true);
                Command::none()
            }
            Message::PlayMacro(macro_name) => {
                let socket_path = self.socket_path.clone();
                let name = macro_name.clone();
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.test_macro(&name).await.map(|_| name).map_err(|e| e.to_string())
                    },
                    Message::MacroPlayed,
                )
            }
            Message::MacroPlayed(Ok(name)) => {
                self.add_notification(&format!("Played macro: {}", name), false);
                Command::none()
            }
            Message::MacroPlayed(Err(e)) => {
                self.add_notification(&format!("Failed to play: {}", e), true);
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
                    self.add_notification("Enter a macro name first", true);
                    return Command::none();
                }
                if self.grabbed_devices.is_empty() {
                    self.add_notification("Grab a device first", true);
                    return Command::none();
                }

                let device_path = self.grabbed_devices.iter().next().unwrap().clone();
                let socket_path = self.socket_path.clone();
                let macro_name = self.new_macro_name.clone();
                self.recording = true;
                self.recording_macro_name = Some(macro_name.clone());

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
                self.add_notification(&format!("Recording '{}' - Press keys now!", name), false);
                Command::none()
            }
            Message::RecordingStarted(Err(e)) => {
                self.recording = false;
                self.recording_macro_name = None;
                self.add_notification(&format!("Failed to start recording: {}", e), true);
                Command::none()
            }
            Message::StopRecording => {
                let socket_path = self.socket_path.clone();
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
                self.add_notification(&format!("Recorded macro: {}", name), false);
                Command::none()
            }
            Message::RecordingStopped(Err(e)) => {
                self.recording = false;
                self.recording_macro_name = None;
                self.add_notification(&format!("Recording failed: {}", e), true);
                Command::none()
            }
            Message::DeleteMacro(macro_name) => {
                let socket_path = self.socket_path.clone();
                let name = macro_name.clone();
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
                self.add_notification(&format!("Deleted: {}", name), false);
                Command::none()
            }
            Message::MacroDeleted(Err(e)) => {
                self.add_notification(&format!("Delete failed: {}", e), true);
                Command::none()
            }
            Message::SaveProfile => {
                if self.profile_name.trim().is_empty() {
                    self.add_notification("Enter a profile name", true);
                    return Command::none();
                }
                let socket_path = self.socket_path.clone();
                let name = self.profile_name.clone();
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.save_profile(&name).await.map_err(|e| e.to_string())
                    },
                    Message::ProfileSaved,
                )
            }
            Message::ProfileSaved(Ok((name, count))) => {
                self.add_notification(&format!("Saved '{}' ({} macros)", name, count), false);
                Command::none()
            }
            Message::ProfileSaved(Err(e)) => {
                self.add_notification(&format!("Save failed: {}", e), true);
                Command::none()
            }
            Message::LoadProfile => {
                if self.profile_name.trim().is_empty() {
                    self.add_notification("Enter a profile name to load", true);
                    return Command::none();
                }
                let socket_path = self.socket_path.clone();
                let name = self.profile_name.clone();
                Command::perform(
                    async move {
                        let client = crate::ipc::IpcClient::new(socket_path);
                        client.load_profile(&name).await.map_err(|e| e.to_string())
                    },
                    Message::ProfileLoaded,
                )
            }
            Message::ProfileLoaded(Ok((name, count))) => {
                self.add_notification(&format!("Loaded '{}' ({} macros)", name, count), false);
                Command::perform(async { Message::LoadMacros }, |msg| msg)
            }
            Message::ProfileLoaded(Err(e)) => {
                self.add_notification(&format!("Load failed: {}", e), true);
                Command::none()
            }
            Message::TickAnimations => {
                let now = Instant::now();
                self.recently_updated_macros.retain(|_, timestamp| {
                    now.duration_since(*timestamp) < Duration::from_secs(3)
                });
                self.recording_pulse = !self.recording_pulse;
                // Auto-dismiss old notifications
                while let Some(notif) = self.notifications.front() {
                    if now.duration_since(notif.timestamp) > Duration::from_secs(5) {
                        self.notifications.pop_front();
                    } else {
                        break;
                    }
                }
                Command::none()
            }
            Message::GrabDevice(device_path) => {
                let socket_path = self.socket_path.clone();
                let path_clone = device_path.clone();
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
                self.add_notification("Device grabbed - ready for recording", false);
                Command::none()
            }
            Message::DeviceGrabbed(Err(e)) => {
                self.add_notification(&format!("Grab failed: {}", e), true);
                Command::none()
            }
            Message::DeviceUngrabbed(Ok(device_path)) => {
                self.grabbed_devices.remove(&device_path);
                self.add_notification("Device released", false);
                Command::none()
            }
            Message::DeviceUngrabbed(Err(e)) => {
                self.add_notification(&format!("Release failed: {}", e), true);
                Command::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let sidebar = self.view_sidebar();
        let main_content = self.view_main_content();
        let status_bar = self.view_status_bar();

        let main_layout = row![
            sidebar,
            vertical_rule(1),
            column![
                main_content,
                horizontal_rule(1),
                status_bar,
            ]
            .height(Length::Fill)
        ];

        container(main_layout)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_millis(500)).map(|_| Message::TickAnimations)
    }
}

impl State {
    fn add_notification(&mut self, message: &str, is_error: bool) {
        self.notifications.push_back(Notification {
            message: message.to_string(),
            is_error,
            timestamp: Instant::now(),
        });
        self.status = message.to_string();
        self.status_history.push_back(message.to_string());
        if self.status_history.len() > 10 {
            self.status_history.pop_front();
        }
        if self.notifications.len() > 5 {
            self.notifications.pop_front();
        }
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let logo = column![
            text("◢").size(40),
            text("RAZERMAPPER").size(16),
            text("v0.2.0").size(10),
        ]
        .spacing(2)
        .align_items(Alignment::Center)
        .width(Length::Fill);

        let nav_button = |label: &str, icon: &str, tab: Tab| {
            let is_active = self.active_tab == tab;
            let btn_style = if is_active {
                iced::theme::Button::Primary
            } else {
                iced::theme::Button::Text
            };

            button(
                row![
                    text(icon).size(18),
                    Space::with_width(10),
                    text(label).size(14),
                ]
                .align_items(Alignment::Center)
            )
            .on_press(Message::SwitchTab(tab))
            .style(btn_style)
            .padding([12, 20])
            .width(Length::Fill)
        };

        let connection_status = if self.daemon_connected {
            row![
                text("●").size(12),
                Space::with_width(8),
                text("Connected").size(11),
            ]
        } else {
            row![
                text("○").size(12),
                Space::with_width(8),
                text("Disconnected").size(11),
            ]
        }
        .align_items(Alignment::Center);

        let sidebar_content = column![
            logo,
            Space::with_height(30),
            nav_button("Devices", "🎮", Tab::Devices),
            nav_button("Macros", "⚡", Tab::Macros),
            nav_button("Profiles", "📁", Tab::Profiles),
            Space::with_height(Length::Fill),
            horizontal_rule(1),
            Space::with_height(10),
            connection_status,
            Space::with_height(5),
            button("Refresh")
                .on_press(Message::CheckDaemonConnection)
                .style(iced::theme::Button::Text)
                .width(Length::Fill),
        ]
        .spacing(4)
        .padding(16)
        .align_items(Alignment::Center);

        container(sidebar_content)
            .width(180)
            .height(Length::Fill)
            .into()
    }

    fn view_main_content(&self) -> Element<'_, Message> {
        let content = match self.active_tab {
            Tab::Devices => self.view_devices_tab(),
            Tab::Macros => self.view_macros_tab(),
            Tab::Profiles => self.view_profiles_tab(),
        };

        container(scrollable(content))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(24)
            .into()
    }

    fn view_devices_tab(&self) -> Element<'_, Message> {
        let header = row![
            text("DEVICES").size(24),
            Space::with_width(Length::Fill),
            button("Reload")
                .on_press(Message::LoadDevices)
                .style(iced::theme::Button::Secondary),
        ]
        .align_items(Alignment::Center);

        let device_list = if self.devices.is_empty() {
            column![
                Space::with_height(40),
                text("No devices found").size(16),
                Space::with_height(10),
                text("Click 'Reload' to scan for input devices").size(12),
            ]
            .align_items(Alignment::Center)
            .width(Length::Fill)
        } else {
            let mut list: Column<Message> = column![].spacing(12);
            for (idx, device) in self.devices.iter().enumerate() {
                list = list.push(self.view_device_card(device, idx));
            }
            list
        };

        column![
            header,
            Space::with_height(20),
            device_list,
        ]
        .spacing(10)
        .into()
    }

    fn view_device_card(&self, device: &DeviceInfo, idx: usize) -> Element<'_, Message> {
        let device_path = device.path.to_string_lossy().to_string();
        let is_grabbed = self.grabbed_devices.contains(&device_path);
        let is_selected = self.selected_device == Some(idx);

        let icon = if device.name.to_lowercase().contains("mouse") {
            "🖱️"
        } else if device.name.to_lowercase().contains("keyboard") {
            "⌨️"
        } else {
            "🎮"
        };

        let status_badge = if is_grabbed {
            container(
                text("GRABBED").size(10)
            )
            .padding([4, 8])
            .style(iced::theme::Container::Box)
        } else {
            container(text("").size(10))
        };

        let action_button = if is_grabbed {
            button("Release")
                .on_press(Message::UngrabDevice(device_path.clone()))
                .style(iced::theme::Button::Destructive)
        } else {
            button("Grab Device")
                .on_press(Message::GrabDevice(device_path.clone()))
                .style(iced::theme::Button::Primary)
        };

        let select_indicator = if is_selected { "▶ " } else { "" };

        let card_content = column![
            row![
                text(icon).size(28),
                Space::with_width(12),
                column![
                    text(format!("{}{}", select_indicator, device.name)).size(16),
                    text(format!(
                        "VID:{:04X} PID:{:04X} | {}",
                        device.vendor_id, device.product_id, device_path
                    )).size(11),
                ],
                Space::with_width(Length::Fill),
                status_badge,
            ]
            .align_items(Alignment::Center),
            Space::with_height(12),
            row![
                button("Select")
                    .on_press(Message::SelectDevice(idx))
                    .style(iced::theme::Button::Text),
                Space::with_width(Length::Fill),
                action_button,
            ]
        ]
        .spacing(8);

        container(card_content)
            .padding(16)
            .width(Length::Fill)
            .style(iced::theme::Container::Box)
            .into()
    }

    fn view_macros_tab(&self) -> Element<'_, Message> {
        let header = row![
            text("MACROS").size(24),
            Space::with_width(Length::Fill),
            text(format!("{} total", self.macros.len())).size(14),
        ]
        .align_items(Alignment::Center);

        let recording_section = self.view_recording_panel();
        let macro_list = self.view_macro_list();

        column![
            header,
            Space::with_height(20),
            recording_section,
            Space::with_height(20),
            text("MACRO LIBRARY").size(18),
            Space::with_height(10),
            macro_list,
        ]
        .spacing(10)
        .into()
    }

    fn view_recording_panel(&self) -> Element<'_, Message> {
        let name_input = text_input("Enter macro name (e.g., 'Quick Reload')", &self.new_macro_name)
            .on_input(Message::UpdateMacroName)
            .padding(12)
            .size(14);

        let record_button = if self.recording {
            let indicator = if self.recording_pulse { "●" } else { "○" };
            button(
                row![
                    text(indicator).size(18),
                    Space::with_width(8),
                    text("STOP RECORDING").size(14),
                ]
                .align_items(Alignment::Center)
            )
            .on_press(Message::StopRecording)
            .style(iced::theme::Button::Destructive)
            .padding([14, 24])
        } else {
            button(
                row![
                    text("⏺").size(18),
                    Space::with_width(8),
                    text("START RECORDING").size(14),
                ]
                .align_items(Alignment::Center)
            )
            .on_press(Message::StartRecording)
            .style(iced::theme::Button::Primary)
            .padding([14, 24])
        };

        let instructions = column![
            text("Recording Instructions").size(14),
            Space::with_height(8),
            text("1. Go to Devices tab and grab a device").size(12),
            text("2. Enter a descriptive macro name above").size(12),
            text("3. Click 'Start Recording' and press keys").size(12),
            text("4. Click 'Stop Recording' when finished").size(12),
        ]
        .spacing(4);

        let recording_status = if self.recording {
            container(
                row![
                    text("●").size(14),
                    Space::with_width(8),
                    text(format!(
                        "Recording '{}' - Press keys on grabbed device...",
                        self.recording_macro_name.as_deref().unwrap_or("")
                    )).size(13),
                ]
                .align_items(Alignment::Center)
            )
            .padding(12)
            .width(Length::Fill)
            .style(iced::theme::Container::Box)
        } else {
            container(text(""))
        };

        let panel_content = column![
            text("MACRO RECORDING").size(16),
            Space::with_height(16),
            name_input,
            Space::with_height(16),
            instructions,
            Space::with_height(16),
            recording_status,
            Space::with_height(16),
            container(record_button).center_x(),
        ];

        container(panel_content)
            .padding(20)
            .width(Length::Fill)
            .style(iced::theme::Container::Box)
            .into()
    }

    fn view_macro_list(&self) -> Element<'_, Message> {
        if self.macros.is_empty() {
            return container(
                column![
                    text("No macros yet").size(14),
                    text("Record your first macro above").size(12),
                ]
                .spacing(8)
                .align_items(Alignment::Center)
            )
            .padding(20)
            .width(Length::Fill)
            .center_x()
            .into();
        }

        let mut list: Column<Message> = column![].spacing(8);

        for macro_entry in &self.macros {
            let is_recent = self.recently_updated_macros.contains_key(&macro_entry.name);
            let name_prefix = if is_recent { "★ " } else { "⚡ " };

            let macro_card = container(
                row![
                    column![
                        text(format!("{}{}", name_prefix, macro_entry.name)).size(15),
                        text(format!(
                            "{} actions | {} trigger keys | {}",
                            macro_entry.actions.len(),
                            macro_entry.trigger.keys.len(),
                            if macro_entry.enabled { "enabled" } else { "disabled" }
                        )).size(11),
                    ]
                    .spacing(4),
                    Space::with_width(Length::Fill),
                    button("▶ Test")
                        .on_press(Message::PlayMacro(macro_entry.name.clone()))
                        .style(iced::theme::Button::Secondary),
                    button("🗑")
                        .on_press(Message::DeleteMacro(macro_entry.name.clone()))
                        .style(iced::theme::Button::Destructive),
                ]
                .spacing(8)
                .align_items(Alignment::Center)
            )
            .padding(12)
            .width(Length::Fill)
            .style(iced::theme::Container::Box);

            list = list.push(macro_card);
        }

        scrollable(list).height(300).into()
    }

    fn view_profiles_tab(&self) -> Element<'_, Message> {
        let header = text("PROFILES").size(24);

        let profile_input = text_input("Profile name...", &self.profile_name)
            .on_input(Message::UpdateProfileName)
            .padding(12)
            .size(14);

        let save_button = button(
            row![
                text("💾").size(16),
                Space::with_width(8),
                text("Save Profile").size(14),
            ]
            .align_items(Alignment::Center)
        )
        .on_press(Message::SaveProfile)
        .style(iced::theme::Button::Primary)
        .padding([12, 20]);

        let load_button = button(
            row![
                text("📂").size(16),
                Space::with_width(8),
                text("Load Profile").size(14),
            ]
            .align_items(Alignment::Center)
        )
        .on_press(Message::LoadProfile)
        .style(iced::theme::Button::Secondary)
        .padding([12, 20]);

        let profile_info = column![
            text("Current Configuration").size(16),
            Space::with_height(10),
            text(format!("• {} devices detected", self.devices.len())).size(12),
            text(format!("• {} devices grabbed", self.grabbed_devices.len())).size(12),
            text(format!("• {} macros configured", self.macros.len())).size(12),
        ]
        .spacing(4);

        let panel_content = column![
            text("SAVE / LOAD CONFIGURATION").size(16),
            Space::with_height(16),
            profile_input,
            Space::with_height(16),
            row![
                save_button,
                Space::with_width(10),
                load_button,
            ],
            Space::with_height(20),
            profile_info,
        ];

        column![
            header,
            Space::with_height(20),
            container(panel_content)
                .padding(20)
                .width(Length::Fill)
                .style(iced::theme::Container::Box),
        ]
        .spacing(10)
        .into()
    }

    fn view_status_bar(&self) -> Element<'_, Message> {
        let connection_indicator = if self.daemon_connected {
            text("● Connected").size(12)
        } else {
            text("○ Disconnected").size(12)
        };

        let latest_notification = if let Some(notif) = self.notifications.back() {
            if notif.is_error {
                text(&notif.message).size(12)
            } else {
                text(&notif.message).size(12)
            }
        } else {
            text("Ready").size(12)
        };

        container(
            row![
                connection_indicator,
                text(" | ").size(12),
                latest_notification,
                Space::with_width(Length::Fill),
                text(format!("{} macros", self.macros.len())).size(12),
            ]
            .spacing(5)
            .align_items(Alignment::Center)
        )
        .padding([8, 16])
        .width(Length::Fill)
        .into()
    }
}
