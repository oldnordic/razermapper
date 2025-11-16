//! RazerMapper GUI Library
//!
//! This library exposes the main GUI components for testing and reuse.

pub mod ipc;
pub mod gui;

// Re-export main types for easier access
pub use gui::{State, Message};