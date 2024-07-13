//! Help utility to fork process

use std::{
    os::fd::{AsRawFd, OwnedFd},
    path::PathBuf,
};

use nix::{
    sys::{
        signal::Signal,
        signalfd::{SfdFlags, SigSet, SignalFd},
    },
    unistd::{ForkResult, Pid},
};
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
        Self {
            subscriber: Some(Box::new(subscriber)),
            cwd: None,
        }
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

        // make a pipe to wait for grandchild's pid
        let (rx, tx) = nix::unistd::pipe()?;

        // fork #1
        match unsafe { nix::unistd::fork()? } {
            ForkResult::Child => {
                let exit_code = match self.run_child(sfd, tx, f) {
                    Ok(_) => 0,
                    Err(error) => {
                        tracing::error!(%error, "unable to daemonize");
                        -1
                    }
                };

                std::thread::sleep(std::time::Duration::from_secs(1));
                std::process::exit(exit_code);
            }
            ForkResult::Parent { child: _ } => {
                sigmask.thread_unblock()?;

                // read the pid of the grandchild
                let mut buf = [0u8; 4];
                nix::unistd::read(rx.as_raw_fd(), &mut buf)?;
                let pid = i32::from_le_bytes(buf);

                return Ok(Pid::from_raw(pid));
            }
        }
    }

    /// Runs the first child
    ///
    /// ### Arguments
    /// * `f` - Function to execute in the child process
    fn run_child<F: FnOnce(SignalFd) -> anyhow::Result<()>>(
        self,
        sfd: SignalFd,
        tx: OwnedFd,
        f: F,
    ) -> anyhow::Result<()> {
        // create a new session and set the process group id
        nix::unistd::setsid()?;

        // fork #2, make sure we can't acquire a controlling terminal
        match unsafe { nix::unistd::fork()? } {
            ForkResult::Child => self.run_grandchild(sfd, f)?,
            ForkResult::Parent { child } => {
                let pid = child.as_raw().to_le_bytes();

                nix::unistd::write(&tx, &pid)?;

                // kill the parent
                std::process::exit(0);
            }
        }

        // only child at this point

        Ok(())
    }

    /// Runs the grandchild process, aka the work we care about
    fn run_grandchild<F: FnOnce(SignalFd) -> anyhow::Result<()>>(
        self,
        sfd: SignalFd,
        f: F,
    ) -> anyhow::Result<()> {
        if let Some(subscriber) = self.subscriber {
            tracing::subscriber::set_global_default(subscriber).ok();
        }

        if let Some(cwd) = self.cwd {
            std::env::set_current_dir(cwd).ok();
        }

        f(sfd)?;

        Ok(())
    }
}
