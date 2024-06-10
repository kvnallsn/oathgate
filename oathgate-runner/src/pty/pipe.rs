//! Unix Pipe PTY

use std::{
    os::fd::{AsRawFd, BorrowedFd},
    process::Child,
    sync::Arc,
};

use mio::{
    event::Source,
    unix::pipe::{Receiver, Sender},
};
use parking_lot::RwLock;
use tui_term::vt100;

use super::OathgatePty;
pub struct PipePty {
    parser: Arc<RwLock<vt100::Parser>>,
    stdout: Receiver,
    stdin: Sender,
}

impl Source for PipePty {
    fn register(
        &mut self,
        registry: &mio::Registry,
        token: mio::Token,
        interests: mio::Interest,
    ) -> std::io::Result<()> {
        self.stdout.register(registry, token, interests)
    }

    fn reregister(
        &mut self,
        registry: &mio::Registry,
        token: mio::Token,
        interests: mio::Interest,
    ) -> std::io::Result<()> {
        self.stdout.reregister(registry, token, interests)
    }

    fn deregister(&mut self, registry: &mio::Registry) -> std::io::Result<()> {
        self.stdout.deregister(registry)
    }
}

impl OathgatePty for PipePty {
    fn pty(&self) -> Arc<RwLock<vt100::Parser>> {
        Arc::clone(&self.parser)
    }

    fn read_pty(&self, buf: &mut [u8]) -> std::io::Result<()> {
        let sz = nix::unistd::read(self.stdout.as_raw_fd(), buf)?;
        self.parser.write().process(&buf[..sz]);
        Ok(())
    }

    fn write_pty(&self, data: &[u8]) -> std::io::Result<()> {
        let fd = unsafe { BorrowedFd::borrow_raw(self.stdin.as_raw_fd()) };
        let sz = nix::unistd::write(&fd, data)?;
        tracing::debug!("wrote {sz} bytes to stdin: {:02x?}", data);
        Ok(())
    }
}

impl PipePty {
    pub fn new(child: &mut Child) -> std::io::Result<Self> {
        let parser = Arc::new(RwLock::new(vt100::Parser::new(24, 80, 0)));

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("stdout missing"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("stdin missing"))?;
        Ok(Self {
            parser,
            stdout: Receiver::from(stdout),
            stdin: Sender::from(stdin),
        })
    }
}
