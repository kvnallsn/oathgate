//! Hypervisor / vm monitor

pub mod config;
pub mod hypervisor;
pub mod pty;
pub mod tui;

pub type Error = Box<dyn std::error::Error + 'static>;
