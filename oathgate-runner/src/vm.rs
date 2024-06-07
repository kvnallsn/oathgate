//! VM-related functions

use std::{
    io::{self, Write},
    path::Path,
    process::{Child, ExitStatus, Stdio},
};

use mio::unix::pipe::Receiver;
use oathgate_net::types::MacAddress;

use crate::MachineConfig;

macro_rules! cmd {
    ($cmd:expr, $($arg:expr),+) => {{
        let mut cmd = std::process::Command::new($cmd);
        $(cmd.arg($arg);)+
        cmd
    }}
}

pub struct VmHandle {
    child: Child,
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
            format!(
                "id=root,file={},if=virtio",
                machine.disk.display()
            ),
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

        Ok(VmHandle { child })
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

    pub fn write_pty(&mut self, buf: &[u8]) -> io::Result<()> {
        if let Some(ref mut stdin) = self.child.stdin {
            stdin.write_all(buf)?;
        }

        Ok(())
    }
}
