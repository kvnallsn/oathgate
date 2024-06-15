//! TTY functions

use std::{
    io, os::fd::{AsRawFd, OwnedFd, RawFd}, thread::JoinHandle
};

use mio::{unix::SourceFd, Events, Interest, Poll, Token};
use nix::{
    ioctl_write_ptr_bad,
    libc::TIOCSWINSZ,
    pty::{forkpty, ForkptyResult, Winsize},
    sys::{signal::Signal, socket::{MsgFlags, Shutdown}},
};

use crate::ErrorContext;

/// Performs a non-blocking recv call, returning 0 if it would block (i.e., EAGAIN / EWOULDBLOCK)
macro_rules! non_blocking_recv {
    ($fd:expr, $buf:expr) => {{
        match nix::sys::socket::recv($fd, $buf, nix::sys::socket::MsgFlags::MSG_DONTWAIT) {
            Ok(sz) => Ok(sz),
            Err(nix::errno::Errno::EWOULDBLOCK) => Ok(0),
            Err(error) => Err(error),
        }
    }};
}

/// A `SockTTY` represents a TTY connected over a socket
pub struct SockTTY {
    client: RawFd,
    tty: OwnedFd,
}

pub enum SocketAction {
    WriteToTTY,
    WouldBlock,
    Continue,
}

impl SockTTY {
    pub fn spawn(client: RawFd, cmd: &str) -> Result<JoinHandle<()>, super::Error> {
        let cmd = std::ffi::CString::new(cmd)?;
        let env = ["TERM=xterm-256color"]
            .into_iter()
            .filter_map(|v| std::ffi::CString::new(v).ok())
            .collect::<Vec<_>>();

        match unsafe { forkpty(None, None) }? {
            ForkptyResult::Child => {
                let args: [std::ffi::CString; 1] = [cmd.clone()];
                nix::unistd::execve(&cmd, &args, &env)?;

                tracing::error!("execve returned!");
                std::process::exit(-1);
            }
            ForkptyResult::Parent { child, master } => {
                let stty = Self {
                    client,
                    tty: master,
                };

                let handle = std::thread::Builder::new()
                    .name(String::from("tty-thread"))
                    .spawn(move || {
                        if let Err(error) = stty.run() {
                            tracing::warn!(
                                error = %error.source(),
                                context = %error.context_str(),
                                %child,
                                "tty died"
                            );
                        }

                        drop(stty);

                        // shutdown socket and reap the child
                        tracing::debug!(%child, "attempting to reap child");
                        match nix::sys::socket::shutdown(client, Shutdown::Both)
                            .and_then(|_| nix::sys::signal::kill(child, Signal::SIGKILL))
                            .and_then(|_| nix::sys::wait::waitpid(child, None))
                        {
                            Ok(_) => tracing::debug!(%child, "child reaped"),
                            Err(error) => tracing::warn!(?error, "unable to reap child"),
                        }
                    })?;

                Ok(handle)
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

        poller
            .registry()
            .register(&mut SourceFd(&self.client), TOKEN_SOCK, Interest::READABLE)?;

        poller.registry().register(
            &mut SourceFd(&self.tty.as_raw_fd()),
            TOKEN_TTY,
            Interest::READABLE,
        )?;

        let mut buf = [0u8; BUFFER_SIZE];
        let mut msg = Vec::with_capacity(BUFFER_SIZE * 4);
        loop {
            poller.poll(&mut events, None).context("poll failed")?;

            for event in &events {
                match event.token() {
                    TOKEN_SOCK => 'sock: loop {
                        match self.read_from_socket(&mut buf, &mut msg).context("read from tty failed")? {
                            SocketAction::WouldBlock => break 'sock,
                            SocketAction::Continue => (),
                            SocketAction::WriteToTTY => {
                                tracing::trace!("read socket msg: {:02x?}", msg);
                                self.write_to_tty(&msg).context("write to tty failed")?;
                            }
                        }
                    },
                    TOKEN_TTY => {
                        self.read_from_tty(&mut buf, &mut msg).context("read from tty failed")?;
                        tracing::trace!("read tty msg: {:02x?}", msg);
                        self.write_to_socket(&msg).context("write to vsock failed")?;
                    }
                    Token(token) => tracing::debug!(%token, "unknown mio token"),
                }
            }
        }
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
    pub fn read_from_socket(&self, buf: &mut [u8], msg: &mut Vec<u8>) -> io::Result<SocketAction> {
        const MSG_TY_ZERO: u8 = 0x00;
        const MSG_TY_DATA: u8 = 0x01;
        const MSG_TY_TTYRESIZE: u8 = 0x02;

        msg.clear();

        let mut msg_type = [0u8];
        non_blocking_recv!(self.client, &mut msg_type)?;
        tracing::trace!("got msg type: {:02x}", msg_type[0]);

        match msg_type[0] {
            MSG_TY_ZERO => Ok(SocketAction::WouldBlock),
            MSG_TY_DATA => {
                self.handle_socket_data(buf, msg)?;
                Ok(SocketAction::WriteToTTY)
            }
            MSG_TY_TTYRESIZE => {
                self.handle_tty_resize()?;
                Ok(SocketAction::Continue)
            }
            msgtype => {
                tracing::warn!(msgtype, "unknown message type");
                Ok(SocketAction::Continue)
            }
        }
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

    /// Handles a TTY resize (i.e., 0x02) message
    ///
    /// Format:
    /// | rows (u16) | cols (u16) |
    fn handle_tty_resize(&self) -> io::Result<()> {
        ioctl_write_ptr_bad!(tiocswinsz, TIOCSWINSZ, Winsize);

        let mut buf = [0u8; 4];
        non_blocking_recv!(self.client, &mut buf)?;

        let rows = u16::from_le_bytes([buf[0], buf[1]]);
        let cols = u16::from_le_bytes([buf[2], buf[3]]);

        if rows == 0 || cols == 0 {
            return Err(io::Error::other("invalid resize message"))?;
        }

        let winsz = Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        tracing::debug!(rows, cols, "attempting to resize tty");
        unsafe {
            tiocswinsz(self.tty.as_raw_fd(), &winsz as *const Winsize)?;
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
