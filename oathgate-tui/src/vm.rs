//! VM-related functions

use std::{io, path::Path, process::{Child, Stdio}};

use crate::MachineConfig;

macro_rules! cmd {
    ($cmd:expr, $($arg:expr),+) => {{
        let mut cmd = std::process::Command::new($cmd);
        $(cmd.arg($arg);)+
        cmd
    }}
}

pub fn run<P: AsRef<Path>>(socket: P, machine: MachineConfig) -> io::Result<Child> {
    let socket = socket.as_ref();

    let mut cmd = cmd!(
            "qemu-system-x86_64",
            "-M", machine.cpu,
            "-enable-kvm",
            "-cpu", "host",
            "-m", &machine.memory,
            "-smp", "1",
            "-kernel", machine.kernel,
            "-append", "earlyprintk=ttyS0 console=ttyS0 root=/dev/vda reboot=k",
            "-nodefaults", "-no-user-config", "-nographic",
            "-serial", "stdio",
            "-object", format!("memory-backend-memfd,id=mem,size={},share=on", machine.memory),
            "-numa", "node,memdev=mem",
            "-drive", format!("id=root,file={},format=raw,if=virtio", machine.disk.display()),
            "-chardev", format!("socket,id=chr0,path={}", socket.display()),
            "-netdev", "type=vhost-user,id=net0,chardev=chr0,queues=1",
            "-device", "virtio-net-pci,netdev=net0"
    );

    cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}
