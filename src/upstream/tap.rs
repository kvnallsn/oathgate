//! an upstream tap device

pub struct Tap {
    name: String,
}

impl Tap {
    /// Creates a new tap device
    ///
    /// Note: This requires administration privileges or CAP_NET_ADMIN
    pub fn create(name: String) -> Self {
        Self { name }
    }

    /// Spawns a new thread to run the i/o of the upstream device
    pub fn spawn(self) {
        //
    }
}
