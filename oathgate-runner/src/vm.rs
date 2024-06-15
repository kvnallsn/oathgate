//! VM-related functions

use std::{
    fmt::Debug,
    io,
    path::Path,
    process::{Child, ExitStatus, Stdio},
};

use mio::unix::pipe::Receiver;
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

    /// Reference to the process running the virtual machine
    child: Child,
}

impl VmHandle {
    pub fn new<P: AsRef<Path>>(socket: P, machine: MachineConfig) -> io::Result<Self> {
        let socket = socket.as_ref();

        let mac = machine.mac.unwrap_or_else(|| MacAddress::generate());
        let bytes = mac.as_bytes();
        let cid = u32::from_be_bytes([0x00, 0x00, bytes[4], bytes[5]]);

        tracing::debug!("launching vm, mac = {mac}, cid = {cid:04x}");

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
            "-chardev",
            format!("socket,id=chr0,path={}", socket.display()),
            "-netdev",
            "type=vhost-user,id=net0,chardev=chr0,queues=1",
            "-device",
            format!("virtio-net-pci,netdev=net0,mac={mac}"),
            "-device",
            format!("vhost-vsock-pci,guest-cid={cid}")
        );

        let child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        Ok(VmHandle { id: cid, child })
    }

    /// Returns the unique ID for this virtual machine.
    ///
    /// Currently, this is the last two bytes of the machine's MAC address
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn stderr(&mut self) -> io::Result<Receiver> {
        self.child
            .stderr
            .take()
            .map(mio::unix::pipe::Receiver::from)
            .ok_or_else(|| io::Error::other("stderr missing"))
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }
}

impl Debug for VmHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VmHandle({:04x})", self.id)
    }
}
