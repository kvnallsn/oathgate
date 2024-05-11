//! File Descriptor Map

use std::{io, path::PathBuf};

use mio::{net::UnixListener, Events, Interest, Poll, Token};

use crate::{device::TapDevice, router::RouterHandle, DeviceOpts};

/// An `FdMap` is a map of unique tokens to file descriptors
pub struct EventPoller {
    socket_path: PathBuf,
    poll: Poll,
}

impl EventPoller {
    /// Creates a new, empty FdMap
    pub fn new<P: Into<PathBuf>>(path: P) -> io::Result<Self> {
        let socket_path = path.into();
        if socket_path.exists() {
            std::fs::remove_file(&socket_path)?;
        }

        let poll = Poll::new()?;
        Ok(Self { socket_path, poll })
    }

    pub fn run(&mut self, device_opts: DeviceOpts, router: RouterHandle) -> io::Result<()> {
        let mut listener = UnixListener::bind(&self.socket_path)?;
        let listener_token = Token(0);
        self.poll
            .registry()
            .register(&mut listener, listener_token, Interest::READABLE)?;

        let mut events = Events::with_capacity(10);
        loop {
            if let Err(error) = self.poll.poll(&mut events, None) {
                tracing::error!(?error, "unable to poll");
                break;
            }

            for event in &events {
                let token = event.token();
                match token {
                    token if token == listener_token => {
                        let (strm, peer) = listener.accept()?;
                        tracing::debug!(?peer, "accepted unix connection");

                        match TapDevice::new(router.clone(), device_opts.clone()) {
                            Ok(dev) => match dev.spawn(strm) {
                                Ok(_) => tracing::debug!("spawned device thread"),
                                Err(error) => {
                                    tracing::warn!(?error, "unable to spawn device thread")
                                }
                            },
                            Err(error) => tracing::warn!(?error, "unable to create tap device"),
                        }
                    }
                    Token(token) => tracing::debug!(?token, "[poller] unknown mio token"),
                }
            }
        }
        Ok(())
    }
}
