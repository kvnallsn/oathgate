//! Process related utilities

use std::fmt::Display;

use anyhow::{anyhow, Context};
use console::{Style, StyledObject};
use nix::{errno::Errno, sys::signal::Signal, unistd::Pid};

#[derive(Clone, Copy, Debug)]
pub enum ProcessState {
    Running(i32),
    PermissionDenied(i32),
    Dead(i32),
    Stopped,
}

/// Helper function to prompt for confirmation and, if approved, stop a process
///
/// ### Arguments
/// * `pid` - Process id of process to stop
pub fn stop(pid: i32) -> anyhow::Result<bool> {
    // send a sigterm to the process
    let pid = Pid::from_raw(pid);

    for i in 0..4 {
        match i {
            3 => nix::sys::signal::kill(pid, Signal::SIGKILL)
                .with_context(|| format!("unable to send sigkill to process {pid}"))?,
            _ => nix::sys::signal::kill(pid, Signal::SIGTERM)
                .with_context(|| format!("unable to send sigterm to process {pid}"))?,
        }

        std::thread::sleep(std::time::Duration::from_millis(1_000));

        match check(pid.as_raw())? {
            ProcessState::Dead(_) | ProcessState::Stopped  => {
                return Ok(true);
            }
            ProcessState::PermissionDenied(_) => {
                return Err(anyhow!("permission denied"));
            }
            _ => (),
        }
    }

    Ok(false)
}

/// Checks to see if a process is alive/accessible.  Returns true if the process is still
/// running, false if it is dead (pid not found) or inaccessible (permissions).  Returns an error
/// in all other cases
///
/// ### Arguments
/// * `pid` - Process id of process to check
pub fn check(pid: i32) -> anyhow::Result<ProcessState> {
    match nix::sys::signal::kill(Pid::from_raw(pid), None) {
        Ok(_) => Ok(ProcessState::Running(pid)),
        Err(Errno::ESRCH) => Ok(ProcessState::Dead(pid)),
        Err(Errno::EPERM) => Ok(ProcessState::PermissionDenied(pid)),
        Err(err) => Err(err.into()),
    }
}

impl ProcessState {
    pub fn styled(self) -> StyledObject<String> {
        let style = match self {
            Self::Running(_) => Style::new().bold().green(),
            Self::PermissionDenied(_) | Self::Dead(_) => Style::new().red().dim(),
            Self::Stopped => Style::new().dim(),
        };

        style.apply_to(self.to_string())
    }

    /// Converts this process state into an option with the pid
    ///
    /// ### Mappings
    /// - `Running` -> Some
    /// - `PermissionDenied` -> Some
    /// - `Dead` -> None
    /// - `Stopped` -> None
    pub fn optional(&self) -> Option<i32> {
        match self {
            Self::Running(pid) => Some(*pid),
            Self::PermissionDenied(pid) => Some(*pid),
            _ => None,
        }
    }
}

impl Display for ProcessState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            Self::Running(_pid) => "running",
            Self::PermissionDenied(_pid) => "permission denied",
            Self::Dead(_) => "dead",
            Self::Stopped => "stopped",
        };

        write!(f, "{msg}")
    }
}
