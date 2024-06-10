//! Pseudo-terminals

mod fabrial;
mod pipe;

use std::sync::Arc;

use mio::event::Source;
use parking_lot::RwLock;
use tui_term::vt100;

pub use self::{fabrial::FabrialPty, pipe::PipePty};

pub trait OathgatePty: Source + Send + Sync {
    fn pty(&self) -> Arc<RwLock<vt100::Parser>>;

    fn read_pty(&self, buf: &mut [u8]) -> std::io::Result<()>;

    fn write_pty(&self, data: &[u8]) -> std::io::Result<()>;

    fn resize_pty(&self, rows: u16, cols: u16) -> std::io::Result<()> {
        self.pty().write().set_size(rows, cols);
        Ok(())
    }
}
