//! RazerMapper GUI Application
//!
//! Main entry point for the RazerMapper GUI application.

mod gui;
mod ipc;

use gui::State;
use iced::Application;

fn main() -> iced::Result {
    tracing_subscriber::fmt::init();
    
    State::run(iced::Settings::default())
}