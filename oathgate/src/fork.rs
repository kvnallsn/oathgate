//! Help utility to fork process

use std::{fs::File, os::fd::AsRawFd, path::PathBuf};

use nix::unistd::{ForkResult, Pid};

/// Configures settings & parameters to use in the newly-forked process
#[derive(Debug, Default)]
pub struct Forker {
    stdout: Option<PathBuf>,
}

impl Forker {
    /// Set the filepath to contain stdout logs
    ///
    /// ### Arguments
    /// * `path` - Path to location to save standard output
    pub fn stdout<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.stdout = Some(path.into());
        self
    }

    /// Execute the fork, returning the PID of the newly spawned child process
    ///
    /// ### Arguments
    /// * `f` - Function to execute in the child process
    pub fn fork<F: FnOnce() -> anyhow::Result<()>>(self, f: F) -> anyhow::Result<Pid> {
        match unsafe { nix::unistd::fork()? } {
            ForkResult::Child => {
                if let Some(stdout) = self.stdout {
                    if let Err(error) = File::options()
                        .append(true)
                        .create(true)
                        .open(stdout)
                        .map_err(anyhow::Error::from)
                        .and_then(|fd| {
                            nix::unistd::dup2(fd.as_raw_fd(), nix::libc::STDOUT_FILENO)
                                .map_err(anyhow::Error::from)
                        }) {
                        tracing::warn!(%error, "unable to redirect stdout to file");
                    }
                }

                match f() {
                    Ok(_) => std::process::exit(0),
                    Err(error) => {
                        tracing::error!(%error, "operation failed");
                        std::process::exit(-1);
                    }
                }
            }
            ForkResult::Parent { child } => {
                return Ok(child);
            }
        }
    }
}
