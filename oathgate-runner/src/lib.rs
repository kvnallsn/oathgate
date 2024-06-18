//! Hypervisor / vm monitor

use std::borrow::Cow;

pub mod config;
pub mod hypervisor;
pub mod pty;
pub mod tui;

#[derive(Debug, thiserror::Error)]
pub enum HypervisorError {
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Errno(#[from] nix::errno::Errno),

    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("{0}")]
    Other(Cow<'static, str>)
}

