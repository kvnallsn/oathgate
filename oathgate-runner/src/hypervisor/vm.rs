//! VM-related functions

use std::{
    fmt::Debug,
    io,
    path::Path,
    process::{Child, Command, Stdio},
};

use oathgate_net::types::MacAddress;

use crate::config::MachineConfig;

macro_rules! cmd {
    ($cmd:expr, $($arg:expr),+) => {{
        let mut cmd = std::process::Command::new($cmd);
        $(cmd.arg($arg);)+
        cmd
    }}
}

/// A `VmHandle` represents a handle to a running virtual machine
pub struct VmHandle {
    /// Unique identifer for this vm.  Currently derived from the last two
    /// bytes of the the MAC address
    id: u32,

    /// Command use to start the virtual machine
    command: Command,
}

impl VmHandle {
    /// Creates a new handle to virtual machine
    ///
    /// ### Arguments
    /// * `sockets` - Path to network bridge socket
    /// * `cid` - Context id of this virtual machine
    /// * `machine` - Machine configuration
    pub fn new<P: AsRef<Path>>(sockets: &[P], cid: u32, machine: MachineConfig) -> io::Result<Self> {
        tracing::debug!("launching vm, cid = {cid:04x}");

        let mut cmd = cmd!(
            "qemu-system-x86_64",
            "-M",
            machine.cpu,
            "-enable-kvm",
            "-cpu",
            "host",
            "-m",
            &machine.memory,
            "-smp",
            "1",
            "-kernel",
            machine.kernel.path(),
            "-append",
            machine.kernel.as_qemu_append("ttyS0"),
            "-nodefaults",
            "-no-user-config",
            "-nographic",
            "-serial",
            "stdio",
            "-object",
            format!(
                "memory-backend-memfd,id=mem,size={},share=on",
                machine.memory
            ),
            "-numa",
            "node,memdev=mem",
            "-drive",
            machine.disk.as_qemu_drive("root"),
            "-device",
            format!("vhost-vsock-pci,guest-cid={cid}")
        );

        for socket in sockets {
            let socket = socket.as_ref(); 
            let mac = MacAddress::generate();

            cmd.arg("-chardev");
            cmd.arg(format!("socket,id=chr0,path={}", socket.display()));
            cmd.arg("-netdev");
            cmd.arg("type=vhost-user,id=net0,chardev=chr0,queues=1");
            cmd.arg("-device");
            cmd.arg(format!("virtio-net-pci,netdev=net0,mac={mac}"));
        }

        cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        Ok(VmHandle { id: cid, command: cmd })
    }

    /// Spawns a new vm, returning the child process information
    pub fn start(&mut self) -> std::io::Result<Child> {
        tracing::debug!(cmd = ?self.command, "starting vm");
        self.command.spawn()
    }

    /// Returns the unique ID for this virtual machine.
    ///
    /// Currently, this is the last two bytes of the machine's MAC address
    pub fn id(&self) -> u32 {
        self.id
    }
}

impl Debug for VmHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VmHandle({:04x})", self.id)
    }
}
