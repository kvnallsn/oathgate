//! Help utility to fork process

use nix::unistd::{ForkResult, Pid};
use tracing::Subscriber;


/// Configures settings & parameters to use in the newly-forked process
#[derive(Default)]
pub struct Forker {
    subscriber: Option<Box<dyn Subscriber + Send + Sync + 'static>>,
}

impl Forker {
    /// Intalls the specified subscriber as tracing's global default for the spawned process
    ///
    /// ### Arguments
    /// * `subscriber` - Tracing subscriber that will catch the emitted events
    pub fn with_subscriber<S: Subscriber + Send + Sync + 'static>(subscriber: S) -> Self {
        Self { subscriber: Some(Box::new(subscriber)) }
    }

    /// Execute the fork, returning the PID of the newly spawned child process
    ///
    /// ### Arguments
    /// * `f` - Function to execute in the child process
    pub fn fork<F: FnOnce() -> anyhow::Result<()>>(self, f: F) -> anyhow::Result<Pid> {
        match unsafe { nix::unistd::fork()? } {
            ForkResult::Child => {
                /*
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
                        // unable to redirect stdout
                    }
                }
                */

                if let Some(subscriber) = self.subscriber {
                    tracing::subscriber::set_global_default(subscriber).ok();
                }

                match f() {
                    Ok(_) => std::process::exit(0),
                    Err(error) => {
                        eprintln!("failed to fork: {error}");
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
