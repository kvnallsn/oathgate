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

    fn resize_pty(&self, _rows: u16, _cols: u16) -> std::io::Result<()> {
        // Default implemention doesn't support resizing
        Ok(())
    }
}
