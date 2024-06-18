//! Terminal map implementation

use std::{collections::HashMap, sync::Arc};

use parking_lot::RwLock;

use crate::pty::OathgatePty;

pub type ArcTerminalMap = Arc<RwLock<TerminalMap>>;

pub struct TerminalMap {
    terminals: HashMap<usize, Box<dyn OathgatePty>>,
    winsz: (u16, u16),
    active: Option<usize>,
}

impl TerminalMap {
    /// Returns a new shared terminal map
    pub fn new() -> ArcTerminalMap {
        Arc::new(RwLock::new(TerminalMap::default()))
    }

    /// Adds a new pty to the terminal map
    ///
    /// ### Arguments
    /// * `token` - Unique id used to identify this pty
    /// * `pty` - Psuedo-terminal linked to virtual machine
    pub fn insert(&mut self, token: usize, pty: Box<dyn OathgatePty>) {
        let (rows, cols) = self.get_size();
        if let Err(error) = pty.resize_pty(rows, cols) {
            tracing::warn!(?error, "unable to resize pty");
        }

        tracing::debug!("created new pty with id {token}");
        self.terminals.insert(token, pty);
    }

    /// Returns a reference to the terminal with the corresponding unique id (token),
    /// or None if one does not exist
    ///
    /// ### Arguments
    /// * `token` - Unique id representing a VM
    pub fn get(&self, token: usize) -> Option<&Box<dyn OathgatePty>> {
        self.terminals.get(&token)
    }

    /// Returns a reference to the active pty if one is set, or `None` if there is
    /// no active pty
    ///
    /// The active pty represents the pty currently being displayed by a TUI
    pub fn get_active(&self) -> Option<&Box<dyn OathgatePty>> {
        self.active.and_then(|id| self.get(id))
    }

    /// Sets the pty with the corresponding id (token) as the active pty
    ///
    /// ### Arguments
    /// * `token` - Unique id of the pty to set as active
    pub fn set_active(&mut self, token: usize) {
        self.active = Some(token);
    }

    /// Helper function to write to the active pty.  If no pty is set as active,
    /// the data is discarded (not written or queued).
    ///
    /// ### Arguments
    /// * `data` - Data to write to the active pty
    pub fn write_to_pty(&self, data: &[u8]) -> std::io::Result<()> {
        match self.get_active() {
            Some(term) => term.write_pty(data),
            None => Ok(()),
        }
    }

    /// Returns an iterator over all ptys currently stored in this terminal map
    pub fn all(&self) -> impl Iterator<Item = &Box<dyn OathgatePty>> {
        self.terminals.values()
    }

    /// Sets the size to use when creating new ptys
    ///
    /// ### Arguments
    /// * `rows` - Number of rows to set in the pty
    /// * `cols` - Number of columns to set in the pty
    pub fn set_size(&mut self, rows: u16, cols: u16) {
        let (old_rows, old_cols) = self.winsz;
        if rows != old_rows || cols != old_cols {
            tracing::debug!(
                "setting default pty size to {rows}x{cols} (was: {old_rows}x{old_cols})"
            );
            self.winsz = (rows, cols);

            // update all terminals, as applicable
            for term in self.all() {
                if let Err(error) = term.resize_pty(rows, cols) {
                    tracing::warn!(?error, "unable to resize terminal");
                }
            }
        }
    }

    /// Returns the current size used to create ptys
    pub fn get_size(&self) -> (u16, u16) {
        self.winsz
    }
}

impl Default for TerminalMap {
    fn default() -> Self {
        Self {
            terminals: HashMap::new(),
            winsz: (24, 80),
            active: None,
        }
    }
}

