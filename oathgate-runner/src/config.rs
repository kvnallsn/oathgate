//! File format / configuration
use std::path::{Path, PathBuf};

use oathgate_net::types::MacAddress;
use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MachineConfig {
    pub cpu: String,
    pub memory: String,
    pub mac: Option<MacAddress>,
    pub kernel: KernelConfig,
    pub disk: DiskConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KernelConfig {
    path: PathBuf,
    root: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiskConfig {
    pub path: PathBuf,
    pub format: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub machine: MachineConfig,
}

impl KernelConfig {
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    pub fn as_qemu_append(&self, tty: &str) -> String {
        let root = self.root.as_str();

        format!("earlyprintk={tty} console={tty} root={root} reboot=k")
    }
}

impl DiskConfig {
    pub fn as_qemu_drive(&self, id: &str) -> String {
        let file = self.path.display();

        match self.format.as_ref() {
            Some(format) => format!("id={id},file={file},format={format},if=virtio"),
            None => format!("id={id},file={file},if=virtio"),
        }
    }
}
