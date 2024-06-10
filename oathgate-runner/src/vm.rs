//! VM-related functions

use std::{
    fmt::Debug, io, os::fd::{AsRawFd, OwnedFd}, path::Path, process::{Child, ExitStatus, Stdio}
};

use mio::unix::pipe::Receiver;
use nix::sys::socket::MsgFlags;
use oathgate_net::types::MacAddress;

use crate::MachineConfig;

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
    id: usize,

    /// Reference to the process running the virtual machine
    child: Child,

    /// Reference to the pty, once this vm is started / running
    pty: Option<OwnedFd>,
}

impl VmHandle {
    pub fn new<P: AsRef<Path>>(socket: P, machine: MachineConfig) -> io::Result<Self> {
        let socket = socket.as_ref();

        let mac = MacAddress::generate();
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
            machine.kernel,
            "-append",
            "earlyprintk=ttyS0 console=ttyS0 root=/dev/vda1 reboot=k",
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
            format!("id=root,file={},if=virtio", machine.disk.display()),
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

        let id = cid as usize;
        Ok(VmHandle { id, child, pty: None })
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn stdout_receiver(&mut self) -> io::Result<Receiver> {
        self.child
            .stdout
            .take()
            .map(mio::unix::pipe::Receiver::from)
            .ok_or_else(|| io::Error::other("stdout missing"))
    }

    pub fn stderr_receiver(&mut self) -> io::Result<Receiver> {
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

    /// Sets the PTY associated with this virtual machine
    ///
    /// ### Arguments
    /// * `fd` - Opened file descriptor to the virtual machine's pty
    pub fn set_pty(&mut self, fd: OwnedFd) {
        self.pty = Some(fd);
    }

    /// Resizes the PTY running the virtual machine to the specified size
    ///
    /// ### Arguments
    /// * `rows` - Number of rows for the terminal emulator
    /// * `cols` - Number of cols for the terminal emulator
    pub fn resize_pty(&self, rows: u16, cols: u16) -> io::Result<()> {
        if let Some(pty) = self.pty.as_ref() {
            tracing::debug!(vmid = self.id, "resizing vm pty to {rows}x{cols}");

            let mut buf = [0x02, 0x00, 0x00, 0x00, 0x00];
            buf[1..3].copy_from_slice(&rows.to_le_bytes());
            buf[3..5].copy_from_slice(&cols.to_le_bytes());
            nix::sys::socket::send(pty.as_raw_fd(), &buf, MsgFlags::empty())?;
        } else {
            tracing::warn!(vmid = self.id, "attempting to resize non-connected pty");
        }

        Ok(())
    }

    /// Writes a message to the PTY
    ///
    /// ### Arugments
    /// * `buf` - Message to write to the PTY
    pub fn write_pty(&mut self, buf: &[u8]) -> io::Result<()> {
        if let Some(pty) = self.pty.as_ref() {
            nix::sys::socket::send(pty.as_raw_fd(), buf, MsgFlags::MSG_DONTWAIT)?;
        }

        Ok(())
    }

    /// Reads data from the virtual machine's pty
    ///
    /// ### Arguments
    /// * `buf` - buffer to read data into
    pub fn read_pty(&self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(pty) = self.pty.as_ref() {
            let sz = nix::sys::socket::recv(pty.as_raw_fd(), buf, MsgFlags::MSG_DONTWAIT)?;
            Ok(sz)
        } else {
            Ok(0)
        }
    }
}

impl Debug for VmHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VmHandle({:04x})", self.id)
    }
}
