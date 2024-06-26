//! Help utility to fork process

use std::path::PathBuf;

use nix::{sys::{signal::Signal, signalfd::{SfdFlags, SigSet, SignalFd}}, unistd::{ForkResult, Pid}};
use tracing::Subscriber;


/// Configures settings & parameters to use in the newly-forked process
#[derive(Default)]
pub struct Forker {
    /// tracing subscriber to log forked processes
    subscriber: Option<Box<dyn Subscriber + Send + Sync + 'static>>,

    /// working directory to use after fork (or none to leave alone)
    cwd: Option<PathBuf>,
}

impl Forker {
    /// Intalls the specified subscriber as tracing's global default for the spawned process
    ///
    /// ### Arguments
    /// * `subscriber` - Tracing subscriber that will catch the emitted events
    pub fn with_subscriber<S: Subscriber + Send + Sync + 'static>(subscriber: S) -> Self {
        Self { subscriber: Some(Box::new(subscriber)), cwd: None }
    }

    /// Sets a new current working directory (aka cwd/pwd) after the fork
    pub fn cwd<P: Into<PathBuf>>(mut self, cwd: P) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Execute the fork, returning the PID of the newly spawned child process
    ///
    /// ### Arguments
    /// * `f` - Function to execute in the child process
    pub fn fork<F: FnOnce(SignalFd) -> anyhow::Result<()>>(self, f: F) -> anyhow::Result<Pid> {
        // block SIGTERM before forking
        let mut sigmask = SigSet::empty();
        sigmask.add(Signal::SIGTERM);
        sigmask.thread_block()?;

        let sfd = SignalFd::with_flags(&sigmask, SfdFlags::SFD_NONBLOCK)?;

        match unsafe { nix::unistd::fork()? } {
            ForkResult::Child => {
                if let Some(subscriber) = self.subscriber {
                    tracing::subscriber::set_global_default(subscriber).ok();
                }

                if let Some(cwd) = self.cwd {
                    std::env::set_current_dir(cwd).ok();
                }

                let exit_code = match f(sfd) {
                    Ok(_) => {
                        tracing::debug!("process exiting");
                        0
                    },
                    Err(error) => {
                        tracing::error!("unable to run process: {error}");
                        tracing::error!("cause: {}", error.root_cause());
                        -1
                    }
                };

                std::thread::sleep(std::time::Duration::from_secs(1));
                std::process::exit(exit_code);
            }
            ForkResult::Parent { child } => {
                sigmask.thread_unblock()?;
                return Ok(child);
            }
        }
    }
}
