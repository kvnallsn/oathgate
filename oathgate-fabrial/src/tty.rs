//! TTY functions

use std::{io, os::fd::{AsRawFd, OwnedFd, RawFd}};

use mio::{unix::SourceFd, Events, Interest, Poll, Token};
use nix::{pty::{forkpty, ForkptyResult}, sys::{signal::Signal, socket::MsgFlags}};

/// Performs a non-blocking recv call, returning 0 if it would block (i.e., EAGAIN / EWOULDBLOCK)
macro_rules! non_blocking_recv {
    ($fd:expr, $buf:expr) => {{
        match nix::sys::socket::recv($fd, $buf, nix::sys::socket::MsgFlags::MSG_DONTWAIT) {
            Ok(sz) => Ok(sz),
            Err(nix::errno::Errno::EWOULDBLOCK) => Ok(0),
            Err(error) => Err(error),
        }
    }}
}

/// A `SockTTY` represents a TTY connected over a socket
pub struct SockTTY {
    client: RawFd,
    tty: OwnedFd,
}

impl SockTTY {
    pub fn spawn(client: RawFd, cmd: &str) -> Result<(), super::Error> {
        let cmd = std::ffi::CString::new(cmd)?;

        match unsafe { forkpty(None, None) }? {
            ForkptyResult::Child => {
                let args: [std::ffi::CString; 1] = [cmd.clone()];
                let env: [std::ffi::CString; 0] = [];
                nix::unistd::execve(&cmd, &args, &env)?;

                tracing::error!("execve returned!");
                std::process::exit(0);
            },
            ForkptyResult::Parent { child, master } => {
                let stty = Self { client, tty: master };

                let _handle = std::thread::Builder::new().name(String::from("tty-thread")).spawn(move || {
                    if let Err(error) = stty.run() {
                        tracing::warn!(?error, %child, "tty died");
                    }

                    // reap the child
                    tracing::debug!(%child, "attempting to reap child");
                    match nix::sys::signal::kill(child, Signal::SIGKILL)
                        .and_then(|_| nix::sys::wait::waitpid(child, None)) {
                        Ok(_) => tracing::debug!(%child, "child reaped"),
                        Err(error) => tracing::warn!(?error, "unable to signal child"),
                    }

                    
                })?;

                Ok(())
            }
        }
    }

    pub fn run(&self) -> Result<(), super::Error> {
        const TOKEN_SOCK: Token = Token(10);
        const TOKEN_TTY: Token = Token(20);
        const MAX_EVENTS: usize = 10;
        const BUFFER_SIZE: usize = 2048;

        let mut poller = Poll::new()?;
        let mut events = Events::with_capacity(MAX_EVENTS);

        poller.registry().register(&mut SourceFd(&self.client), TOKEN_SOCK, Interest::READABLE)?;
        poller.registry().register(&mut SourceFd(&self.tty.as_raw_fd()), TOKEN_TTY, Interest::READABLE)?;

        let mut buf = [0u8; BUFFER_SIZE];
        let mut msg = Vec::with_capacity(BUFFER_SIZE * 4);
        while let Ok(_) = poller.poll(&mut events, None) {
            for event in &events {
                match event.token() {
                    TOKEN_SOCK => {
                        'sock: loop {
                            match self.read_from_socket(&mut buf, &mut msg) {
                                Ok(_) => {
                                    tracing::debug!("read socket msg: {:02x?}", msg);
                                    self.write_to_tty(&msg)?;
                                },
                                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break 'sock,
                                Err(err) => return Err(err)?,
                            }
                        }
                    },
                    TOKEN_TTY => {
                        self.read_from_tty(&mut buf, &mut msg)?;
                        tracing::trace!("read tty msg: {:02x?}", msg);
                        self.write_to_socket(&msg)?;
                    },
                    Token(token) => tracing::debug!(%token, "unknown mio token"),
                }
            }
        }

        Ok(())
    }

    /// Reads a message from the client
    ///
    /// Message structure:
    /// ```ignore
    /// | MsgType (u8) | Payload ... |
    /// ```
    ///
    /// Message Types:
    /// - 0x01: TTY data (aka user input)
    /// - 0x02: Resize TTY
    pub fn read_from_socket(&self, buf: &mut [u8], msg: &mut Vec<u8>) -> io::Result<()> {
        const MSG_TY_ZERO: u8 = 0x00;
        const MSG_TY_DATA: u8 = 0x01;
        const MSG_TY_TTYRESIZE: u8 = 0x02;

        msg.clear();

        let mut msg_type = [0u8];
        non_blocking_recv!(self.client, &mut msg_type)?;
        tracing::trace!("got msg type: {:02x}", msg_type[0]);

        match msg_type[0] {
            MSG_TY_ZERO => {
                return Err(io::Error::new(io::ErrorKind::WouldBlock, "no more data"))?;
            },
            MSG_TY_DATA => {
                self.handle_socket_data(buf, msg)?;
            },
            MSG_TY_TTYRESIZE => (),
            msgtype => tracing::warn!(msgtype, "unknown message type"),
        };

        Ok(())
    }

    /// Handles a data message (aka msg code: 0x01)
    ///
    /// ### Arguments
    /// * `buf` - Buffer to read data into
    /// * `msg` - Message vector to store completed message
    fn handle_socket_data(&self, buf: &mut [u8], msg: &mut Vec<u8>) -> io::Result<()> {
        // read 2 bytes for data size
        let mut length = [0u8; 2];
        non_blocking_recv!(self.client, &mut length)?;
        let length = u16::from_le_bytes(length);
        let mut length = usize::from(length);
        tracing::trace!("expected msg len: {length} bytes");

        if length == 0 {
            return Err(io::Error::other("data length missing for data msg"))?;
        }

        loop {
            let sz = std::cmp::min(length, buf.len());
            if sz == 0 {
                // no more data to read
                break;
            }

            let sz = non_blocking_recv!(self.client, &mut buf[..sz])?;

            match sz {
                0 => break,
                sz => {
                    msg.extend_from_slice(&buf[..sz]);
                    length -= sz;
                }
            }
        }

        Ok(())
    }

    /// Read from the tty device
    ///
    /// ### Arguments
    /// * `buf` - Buffer to read data into
    /// * `msg` - Message to store completed data
    fn read_from_tty(&self, buf: &mut [u8], msg: &mut Vec<u8>) -> io::Result<()> {
        msg.clear();
        let sz = nix::unistd::read(self.tty.as_raw_fd(), buf)?;
        msg.extend_from_slice(&buf[..sz]);
        Ok(())
    }

    /// Writes data to the tty master fd
    ///
    /// ### Arguments
    /// * `data` - Data to write to the tty device
    fn write_to_tty(&self, data: &[u8]) -> io::Result<()> {
        nix::unistd::write(&self.tty, data)?;
        Ok(())
    }

    /// Writes data to the socket
    fn write_to_socket(&self, data: &[u8]) -> io::Result<()> {
        nix::sys::socket::send(self.client, &data, MsgFlags::MSG_DONTWAIT)?;
        Ok(())
    }
}