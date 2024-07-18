//! File format / configuration
use std::{
    fs::File, io::Read, path::{Path, PathBuf}
};

use oathgate_net::types::MacAddress;
use serde::{Deserialize, Serialize};

use crate::HypervisorError;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MachineConfig {
    pub cpu: String,
    pub memory: String,
    pub kernel: KernelConfig,
    pub disk: DiskConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KernelConfig {
    /// Path on host system to kernel image
    pub path: PathBuf,

    /// Root partition/device to attempt to mount (i.e., /dev/vda or /dev/vda1)
    pub root: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiskConfig {
    /// Path on host system to disk image
    pub path: PathBuf,

    /// Format (i.e., raw,qcow) of this disk image
    pub format: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NetDevConfig {
    /// MAC address of this network adapter
    pub mac: Option<MacAddress>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub machine: MachineConfig,
}

impl Config {
    /// Loads a configuration file from a path on disk
    ///
    /// ### Arguments
    /// * `path` - Path to file to load
    pub fn from_yaml<P: AsRef<Path>>(path: P) -> Result<Self, HypervisorError> {
        let fd = File::open(path)?;
        Self::read_yaml(fd)
    }

    /// Loads a configuration file from reader
    ///
    /// ### Arguments
    /// * `rdr` - Reader to read from and deserialize yaml
    pub fn read_yaml<R: Read>(rdr: R) -> Result<Self, HypervisorError> {
        let cfg: Config = serde_yaml::from_reader(rdr)?;
        Ok(cfg)
    }
}

impl MachineConfig {
    /// Creates a new machine configuration
    ///
    /// ### Arguments
    /// * `cpu` - Type of CPU to emulate
    /// * `memory` - Amount of memory to allocate virtual machine
    /// * `kernel` - Kernel configuration
    /// * `disk` - Disk configuration
    pub fn new<C: Into<String>, M: Into<String>>(cpu: C, memory: M, kernel: KernelConfig, disk: DiskConfig) -> Self {
        let cpu = cpu.into();
        let memory = memory.into();

        Self { cpu, memory, kernel, disk }
    }

    /// Loads a configuration file from reader
    ///
    /// ### Arguments
    /// * `rdr` - Reader to read from and deserialize yaml
    pub fn read_yaml<R: Read>(rdr: R) -> Result<Self, HypervisorError> {
        let cfg: MachineConfig = serde_yaml::from_reader(rdr)?;
        Ok(cfg)
    }
}

impl KernelConfig {
    /// Creates a new kernel configuration for an on-disk kernel image
    ///
    /// ### Arguments
    /// * `path` - Path to kernel image on disk
    /// * `root` - Root device to boot (e.g., /dev/vda, /dev/vda1, etc.)
    pub fn new<P: Into<PathBuf>, R: Into<String>>(path: P, root: R) -> Self {
        let path = path.into();
        let root = root.into();

        Self { path, root }
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    pub fn as_qemu_append(&self, tty: &str) -> String {
        let root = self.root.as_str();

        format!("earlyprintk={tty} console={tty} root={root} reboot=k")
    }
}

impl DiskConfig {
    /// Creates a new disk configuration
    ///
    /// ### Arguments
    /// * `path` - Path to the disk
    pub fn new<P: Into<PathBuf>, F: Into<String>>(path: P, format: F) -> Self {
        let path = path.into(); 

        Self { path, format: Some(format.into())}
    }

    pub fn as_qemu_drive(&self, id: &str) -> String {
        let file = self.path.display();

        match self.format.as_ref() {
            Some(format) => format!("id={id},file={file},format={format},if=virtio"),
            None => format!("id={id},file={file},if=virtio"),
        }
    }
}
