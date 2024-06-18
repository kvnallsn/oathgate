//! A simple hypervisor to manage the qemu instance

mod terminal;
mod vm;

use std::{
    os::fd::{AsRawFd, OwnedFd},
    path::Path,
    sync::Arc,
};

use mio::{
    unix::{pipe::Receiver, SourceFd},
    Events, Interest, Poll, Token,
};
use nix::{
    libc::VMADDR_CID_HOST,
    sys::{
        signal::Signal,
        signalfd::{SfdFlags, SigSet, SignalFd},
        socket::{bind, listen, socket, AddressFamily, Backlog, SockFlag, SockType, VsockAddr},
    },
    unistd::Pid,
};

use crate::{
    config::MachineConfig,
    hypervisor::{terminal::TerminalMap, vm::VmHandle},
    pty::{FabrialPty, PipePty},
};

pub use self::terminal::ArcTerminalMap;

/// Maximum number of backlog'd connections waiting to be `accept`ed
const MAX_BACKLOG: i32 = 10;

/// Mio token values
const TOKEN_STDERR: Token = Token(2);
const TOKEN_SIGNAL: Token = Token(3);
const TOKEN_HYPERVISOR: Token = Token(4);

pub struct Hypervisor {
    /// vhost socket listening for connections
    vsock: OwnedFd,

    /// handle to the signal file descriptor
    signal: SignalFd,

    /// Handle to the virtual machine
    vm: VmHandle,

    /// handle to map of active terminals/ptys/ttys
    terminals: ArcTerminalMap,
}

impl Hypervisor {
    /// Creates a new hypervisor bound to the specified vhost port on the hypervisor CID (aka 2)
    ///
    /// ### Arguments
    /// * `port` - Port to bind on vhost socket
    pub fn new<P: AsRef<Path>>(network: P, config: MachineConfig) -> Result<Self, super::Error> {
        let vm = VmHandle::new(network, config)?;

        tracing::debug!(
            "binding hypervisor socket (cid = {}, port = {})",
            VMADDR_CID_HOST,
            vm.id()
        );

        let addr = VsockAddr::new(VMADDR_CID_HOST, vm.id());
        let vsock = socket(
            AddressFamily::Vsock,
            SockType::Stream,
            SockFlag::SOCK_NONBLOCK,
            None,
        )?;
        bind(vsock.as_raw_fd(), &addr)?;
        listen(&vsock, Backlog::new(MAX_BACKLOG)?)?;

        let mut mask = SigSet::empty();
        mask.add(nix::sys::signal::SIGINT);
        mask.add(nix::sys::signal::SIGTERM);
        mask.thread_block()?;

        let signal = SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK)?;

        let terminals = TerminalMap::new();

        Ok(Self {
            vsock,
            signal,
            vm,
            terminals,
        })
    }

    /// Returns a new atomically ref-counted (Arc) instance of the terminal map
    pub fn terminals(&self) -> ArcTerminalMap {
        Arc::clone(&self.terminals)
    }

    /// Runs the hypervisor
    pub fn run(&mut self) -> Result<(), super::Error> {
        let mut poller = Poll::new()?;
        let mut poller_next_id = 10;

        let mut vm = self.vm.start()?;

        let mut stderr = vm.stderr.take().map(|s| Receiver::from(s)).unwrap();

        stderr.set_nonblocking(true)?;

        poller
            .registry()
            .register(&mut stderr, TOKEN_STDERR, Interest::READABLE)?;
        poller.registry().register(
            &mut SourceFd(&self.signal.as_raw_fd()),
            TOKEN_SIGNAL,
            Interest::READABLE,
        )?;
        poller.registry().register(
            &mut SourceFd(&self.vsock.as_raw_fd()),
            TOKEN_HYPERVISOR,
            Interest::READABLE,
        )?;

        // register terminals
        let mut stdio_term = PipePty::new(&mut vm)?;
        poller
            .registry()
            .register(&mut stdio_term, Token(1), Interest::READABLE)?;
        self.terminals.write().insert(1, Box::new(stdio_term));
        self.terminals.write().set_active(1);

        let mut buf = [0u8; 4096];
        let mut events = Events::with_capacity(10);
        'poll: loop {
            poller.poll(&mut events, None)?;

            for event in &events {
                match event.token() {
                    TOKEN_STDERR => stderr.try_io(|| {
                        let sz = nix::unistd::read(stderr.as_raw_fd(), &mut buf)?;
                        let msg = String::from_utf8_lossy(&buf[..sz]);
                        tracing::warn!(%msg, "qemu stderr");

                        Ok(())
                    })?,
                    TOKEN_SIGNAL => match self.signal.read_signal()? {
                        Some(sig) => match Signal::try_from(sig.ssi_signo as i32) {
                            Ok(Signal::SIGINT) => break 'poll,
                            Ok(Signal::SIGTERM) => break 'poll,
                            Ok(signal) => tracing::debug!(%signal, "caught unhandled signal"),
                            Err(error) => tracing::warn!(%error, "unknown signal number"),
                        },
                        None => (),
                    },
                    TOKEN_HYPERVISOR => {
                        let mut vmid = [0u8; 2];
                        let csock = nix::sys::socket::accept(self.vsock.as_raw_fd())?;
                        nix::unistd::read(csock, &mut vmid)?;
                        let vmid = u16::from_le_bytes(vmid);

                        tracing::debug!(%vmid, "vm starting, opening fabrial connection to vm");

                        match FabrialPty::connect(vmid as u32, 3715) {
                            Ok(mut fpty) => {
                                tracing::debug!("fabrial connected, switching ptys");
                                poller.registry().register(
                                    &mut fpty,
                                    Token(poller_next_id),
                                    Interest::READABLE,
                                )?;
                                self.terminals
                                    .write()
                                    .insert(poller_next_id, Box::new(fpty));
                                self.terminals.write().set_active(poller_next_id);
                                poller_next_id = poller_next_id + 1;
                            }
                            Err(error) => tracing::warn!(
                                ?error,
                                "unable to connect to fabrial, is the service running?"
                            ),
                        }
                    }
                    Token(token) => match self.terminals.read().get(token) {
                        None => tracing::debug!(token, "unknown mio token"),
                        Some(term) => {
                            term.read_pty(&mut buf)?;
                        }
                    },
                }
            }
        }

        tracing::debug!("sending SIGTERM to vm");
        nix::sys::signal::kill(Pid::from_raw(vm.id() as i32), Signal::SIGTERM)?;

        tracing::debug!("waiting for vm process to stop");
        drop(stderr);
        vm.wait()?;

        Ok(())
    }
}
