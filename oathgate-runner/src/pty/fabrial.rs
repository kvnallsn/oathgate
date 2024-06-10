//! Fabrial PTY

use std::{io, os::fd::{AsRawFd, OwnedFd}, sync::Arc};

use mio::{event::Source, unix::SourceFd};
use nix::{errno::Errno, sys::socket::{AddressFamily, MsgFlags, SockFlag, VsockAddr}};
use parking_lot::RwLock;
use tui_term::vt100;

use super::OathgatePty;

pub struct FabrialPty {
    sock: OwnedFd,
    parser: Arc<RwLock<vt100::Parser>>,
}

impl FabrialPty {
    /// Connects to a VM with the specified id and port
    ///
    /// ### Arguments
    /// * `cid` - Unique idenifier for VM (control)
    /// * `port` - Port to connect (unique per cid)
    pub fn new(cid: u32, port: u32) -> io::Result<Self> {
        let addr = VsockAddr::new(cid, port);
        let sock = nix::sys::socket::socket(
            AddressFamily::Vsock,
            nix::sys::socket::SockType::Stream,
            SockFlag::empty(),
            None,
        )?;
        nix::sys::socket::connect(sock.as_raw_fd(), &addr)?;

        let parser = Arc::new(RwLock::new(vt100::Parser::new(24, 80, 0)));

        Ok(Self { sock, parser })
    }
}

impl Source for FabrialPty {
    fn register(
        &mut self,
        registry: &mio::Registry,
        token: mio::Token,
        interests: mio::Interest,
    ) -> io::Result<()> {
        registry.register(&mut SourceFd(&self.sock.as_raw_fd()), token, interests)
    }

    fn reregister(
        &mut self,
        registry: &mio::Registry,
        token: mio::Token,
        interests: mio::Interest,
    ) -> io::Result<()> {
        registry.reregister(&mut SourceFd(&self.sock.as_raw_fd()), token, interests)
    }

    fn deregister(&mut self, registry: &mio::Registry) -> io::Result<()> {
        registry.deregister(&mut SourceFd(&self.sock.as_raw_fd()))
    }
}

impl OathgatePty for FabrialPty {
    fn pty(&self) -> Arc<RwLock<vt100::Parser>> {
        Arc::clone(&self.parser)
    }

    fn read_pty(&self, buf: &mut [u8]) -> io::Result<()> {
        let mut parser = self.parser.write();
        loop {
            match nix::sys::socket::recv(self.sock.as_raw_fd(), buf, MsgFlags::MSG_DONTWAIT) {
                Err(Errno::EWOULDBLOCK) => break,
                Err(err) => Err(err)?,
                Ok(sz) => parser.process(&buf[..sz]),
            }
        }

        Ok(())
    }

    fn write_pty(&self, data: &[u8]) -> io::Result<()> {
        // write a data message (code: 0x01)
        let len = data.len().to_le_bytes();
        let hdr = [0x01, len[0], len[1]];
        nix::sys::socket::send(self.sock.as_raw_fd(), &hdr, MsgFlags::empty())?;
        nix::sys::socket::send(self.sock.as_raw_fd(), &data, MsgFlags::empty())?;
        Ok(())
    }

    fn resize_pty(&self, rows: u16, cols: u16) -> io::Result<()> {
        // write a resize message (code: 0x02)
        self.parser.write().set_size(rows, cols);
        let rows = rows.to_le_bytes();
        let cols = cols.to_le_bytes();
        let msg = [0x02, rows[0], rows[1], cols[0], cols[1]];
        nix::sys::socket::send(self.sock.as_raw_fd(), &msg, MsgFlags::empty())?;
        Ok(())
    }
}
