//! Process related utilities

use std::fmt::Display;

use console::{Style, StyledObject};
use dialoguer::Confirm;
use nix::{errno::Errno, sys::signal::Signal, unistd::Pid};

use crate::State;

#[derive(Clone, Copy, Debug)]
pub enum ProcessState {
    Running,
    PermissionDenied,
    Dead,
}

/// Helper function to prompt for confirmation and, if approved, stop a process
///
/// ### Arguments
/// * `state` - Application state
/// * `pid` - Process id of process to stop
/// * `prompt` - Prompt to display in confirmation prompt
pub fn stop(state: &State, pid: i32, prompt: &str) -> anyhow::Result<bool> {
    let confirmation =
        state.skip_confirm() || Confirm::new().with_prompt(prompt).interact()?;

    if confirmation {
        // send a sigterm to the process
        nix::sys::signal::kill(Pid::from_raw(pid), Signal::SIGTERM)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Checks to see if a process is alive/accessible.  Returns true if the process is still
/// running, false if it is dead (pid not found) or inaccessible (permissions).  Returns an error
/// in all other cases
///
/// ### Arguments
/// * `pid` - Process id of process to check
pub fn check(pid: i32) -> anyhow::Result<ProcessState> {
    match nix::sys::signal::kill(Pid::from_raw(pid), None) {
        Ok(_) => Ok(ProcessState::Running),
        Err(Errno::ESRCH) => Ok(ProcessState::Dead),
        Err(Errno::EPERM) => Ok(ProcessState::PermissionDenied),
        Err(err) => Err(err.into()),
    }
}

impl ProcessState {
    pub fn styled(self) -> StyledObject<String> {
        let style = match self {
            Self::Running => Style::new().bold().green(),
            Self::PermissionDenied | Self::Dead => Style::new().bold().red().dim(),
        };

        style.apply_to(self.to_string())
    }
}

impl Display for ProcessState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            Self::Running => "running",
            Self::PermissionDenied => "permission denied",
            Self::Dead => "dead",
        };

        write!(f, "{msg}")
    }
}
