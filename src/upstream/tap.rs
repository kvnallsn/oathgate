//! an upstream tap device

use crate::error::{AppResult, UpstreamError};

/// Maximum length of a network device name
const IFNAMESZ: usize = 16;

pub struct Tap {
    name: String,
}

impl Tap {
    /// Creates a new tap device
    ///
    /// Note: This requires administration privileges or CAP_NET_ADMIN
    pub fn create(name: String) -> AppResult<Self> {
        if name.len() > IFNAMESZ {
            return Err(UpstreamError::CreateFailed(format!(
                "device name ({name}) is too long, max length is {IFNAMESZ}, provided length {}",
                name.len()
            )))?;
        }

        Ok(Self { name })
    }

    /// Spawns a new thread to run the i/o of the upstream device
    pub fn spawn(self) {
        //
    }
}
