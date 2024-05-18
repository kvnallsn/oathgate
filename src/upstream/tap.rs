//! an upstream tap device

use std::{fmt::Debug, fs::File, io::{IoSlice, Read, Write}, net::Ipv4Addr, os::fd::AsRawFd, sync::Arc};

use crate::{error::{AppResult, UpstreamError}, router::RouterHandle};

use flume::{Receiver, Sender};
use mio::{unix::SourceFd, Events, Interest, Poll, Token, Waker};
use nix::{libc::{IFF_NO_PI, IFF_TUN, IFNAMSIZ}, net::if_::if_nametoindex};
use oathgate_net::Ipv4Header;

use super::UpstreamHandle;

/// Maximum number of events mio can processes at one time
const MAX_EVENTS_CAPACITY: usize = 10;

/// Tokens / handles for mio sources
const TOKEN_READ: Token = Token(0);
const TOKEN_WRITE: Token = Token(1);

// FIX: this should be a parameter
const TAP_IPV4: Ipv4Addr = Ipv4Addr::new(10, 213, 100, 1);

pub struct Tun {
    /// Name of the tun device
    name: String,

    /// Opened file descriptor to the device
    fd: File,

    /// Index of the device
    idx: u32
}

pub struct TunHandle {
    tx: Sender<(Ipv4Header, Vec<u8>)>,
    waker: Arc<Waker>,
}

// ifreq is 40 bytes long
#[repr(C)]
#[derive(Default)]
struct IfReqCreateTun {
    ifrn_name: [u8; IFNAMSIZ], // 16 is IFNAMSIZ from linux/if.h
    ifru_flags: u16,
    padding: [u8; 22],
}

impl Tun {
    /// Creates a new tap device
    ///
    /// Note: This requires administration privileges or CAP_NET_ADMIN
    pub fn create(name: String) -> AppResult<Self> {
        // #define TUNSETIFF _IOW('T', 202, int)
        nix::ioctl_write_int!(tunsetiff, b'T', 202);

        // #define TUNSETPERSIST _IOW('T', 203, int)
        //nix::ioctl_write_int!(tunsetpersist, b'T', 203);

        let len = name.len();
        if len > IFNAMSIZ {
            return Err(UpstreamError::CreateFailed(format!(
                "device name ({name}) is too long, max length is {IFNAMSIZ}, provided length {len}",
            )))?;
        }

        let mut ifreq = IfReqCreateTun::default();
        let len = std::cmp::min(IFNAMSIZ, len);
        ifreq.ifrn_name[0..len].copy_from_slice(&name.as_bytes()[0..len]);
        ifreq.ifru_flags = (IFF_TUN | IFF_NO_PI) as u16;

        // Create TAP via ioctls
        let fd = File::options()
            .read(true)
            .write(true)
            .open("/dev/net/tun")?;

        unsafe {
            tunsetiff(fd.as_raw_fd(), (&ifreq as *const _) as u64)?;
            //tunsetpersist(fd.as_raw_fd(), 0x1)?;
        };

        let idx = if_nametoindex(&name.as_bytes()[..len])?;

        Ok(Self { name, fd, idx })
    }

    /// Spawns a new thread to run the i/o of the upstream device
    pub fn spawn(self, router: RouterHandle) -> AppResult<()> {
        let poll = Poll::new()?;
        let waker = Waker::new(poll.registry(), TOKEN_WRITE)?;
        poll.registry().register(&mut SourceFd(&self.fd.as_raw_fd()), TOKEN_READ, Interest::READABLE)?;

        let (tx, rx) = flume::unbounded();
        let handle = TunHandle { tx, waker: Arc::new(waker) };
        let handle = Box::new(handle);

        router.set_upstream(handle);

        std::thread::Builder::new().name(String::from("upstream-tun")).spawn(move || {
            if let Err(error) = self.run(poll, rx) {
                tracing::error!(?error, "unable to run tun device");
            }
        })?;

        Ok(())
    }

    fn run(mut self, mut poll: Poll, rx: Receiver<(Ipv4Header, Vec<u8>)>) -> AppResult<()> {
        let mut events = Events::with_capacity(MAX_EVENTS_CAPACITY);

        loop {
            poll.poll(&mut events, None)?;

            for event in &events {
                match event.token() {
                    TOKEN_READ => match self.read_from_device() {
                        Ok(_) => (),
                        Err(error) => tracing::warn!(?error, "[upstream] unable to read from tun device"),
                    },
                    TOKEN_WRITE => {
                        for (hdr, pkt) in rx.drain() {
                            match self.write_to_device(hdr, pkt) {
                                Ok(()) => tracing::trace!("[upstream] wrote ipv4 packet to tun device"),
                                Err(error) => tracing::error!(?error, "[upstream] unable to write to tun device"),
                            }
                        }
                    }
                    Token(token) => tracing::debug!(token, "[upstream] unknown mio token"),
                }
            }
        }
    }

    fn read_from_device(&mut self) -> AppResult<()> {
        let mut buf = [0u8; 1024];
        let sz = self.fd.read(&mut buf)?;
        tracing::debug!("[upstream] read {sz} bytes");
        Ok(())
    }

    fn write_to_device(&mut self, mut hdr: Ipv4Header, pkt: Vec<u8>) -> AppResult<()> {
        let old = hdr.masquerade(TAP_IPV4);
        tracing::debug!("[upstream] masquerade {:?} -> {:?}", old, TAP_IPV4);

        let hdr = hdr.into_bytes();
        let iovs = [IoSlice::new(&hdr), IoSlice::new(&pkt)];
        let sz = self.fd.write_vectored(&iovs)?;

        tracing::debug!("[upstream] wrote {sz} bytes");

        Ok(())
    }
}

impl Debug for Tun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tun({:02}:{})", self.idx, self.name)
    }
}

impl UpstreamHandle for TunHandle {
    fn write(&self, hdr: Ipv4Header, buf: Vec<u8>) -> Result<(), crate::error::Error> {
        self.tx.send((hdr, buf)).ok();
        self.waker.wake().ok();
        Ok(())
    }
} 
