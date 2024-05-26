//! an upstream tap device

use std::{
    fmt::Debug,
    fs::File,
    io::{IoSlice, Read, Write},
    os::fd::AsRawFd,
    sync::Arc,
};

use crate::{
    error::{AppResult, Error, UpstreamError},
    router::RouterHandle,
};

use flume::{Receiver, Sender};
use mio::{unix::SourceFd, Events, Interest, Poll, Token, Waker};
use nix::{
    libc::{IFF_NO_PI, IFF_TAP, IFF_TUN, IFNAMSIZ, SIOCGIFHWADDR},
    net::if_::if_nametoindex,
};
use oathgate_net::{types::MacAddress, Ipv4Packet};

use super::{Wan, WanHandle};

/// Maximum number of events mio can processes at one time
const MAX_EVENTS_CAPACITY: usize = 10;

/// Tokens / handles for mio sources
const TOKEN_READ: Token = Token(0);
const TOKEN_WRITE: Token = Token(1);

pub struct TunTap {
    /// Name of the tun device
    name: String,

    /// Opened file descriptor to the device
    fd: File,

    /// Index of the device
    idx: u32,

    /// Poller instance to read/write device
    poll: Poll,

    /// Channel used to send packets received from this device
    tx: Sender<Ipv4Packet>,

    /// Channel used to receive packets to send out this device
    rx: Option<Receiver<Ipv4Packet>>,

    /// Mac Address of the device
    mac: MacAddress,
}

pub struct TunTapHandle {
    tx: Sender<Ipv4Packet>,
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

impl TunTap {
    /// Creates a new tap device
    ///
    /// Note: This requires administration privileges or CAP_NET_ADMIN
    pub fn create_tap(name: String) -> AppResult<Self> {
        Self::create(name, IFF_TAP)
    }

    /// Creates a new tun device
    ///
    /// Note: This requires administration privileges or CAP_NET_ADMIN
    #[allow(dead_code)]
    pub fn create_tun(name: String) -> AppResult<Self> {
        Self::create(name, IFF_TUN)
    }

    fn create(name: String, flags: i32) -> AppResult<Self> {
        // #define TUNSETIFF _IOW('T', 202, int)
        nix::ioctl_write_int!(tunsetiff, b'T', 202);

        // #define TUNSETPERSIST _IOW('T', 203, int)
        //nix::ioctl_write_int!(tunsetpersist, b'T', 203);

        // #define SIOCGIFHWADDR 0x8927
        nix::ioctl_read_bad!(siocgifhwaddr, SIOCGIFHWADDR, nix::libc::ifreq);

        let len = name.len();
        if len > IFNAMSIZ {
            return Err(UpstreamError::CreateFailed(format!(
                "device name ({name}) is too long, max length is {IFNAMSIZ}, provided length {len}",
            )))?;
        }

        let mut ifreq = IfReqCreateTun::default();
        let len = std::cmp::min(IFNAMSIZ, len);
        ifreq.ifrn_name[0..len].copy_from_slice(&name.as_bytes()[0..len]);
        ifreq.ifru_flags = (flags | IFF_NO_PI) as u16;

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

        let poll = Poll::new()?;
        let (tx, rx) = flume::unbounded();

        let mac = match flags {
            IFF_TAP => {
                let mut ifr_name = [0i8; IFNAMSIZ];
                for (idx, b) in name.as_bytes().iter().enumerate() {
                    ifr_name[idx] = *b as i8;
                }

                // get the mac address
                let mut req = nix::libc::ifreq {
                    ifr_name,
                    ifr_ifru: nix::libc::__c_anonymous_ifr_ifru {
                        ifru_hwaddr: nix::libc::sockaddr {
                            sa_family: 0,
                            sa_data: [0; 14],
                        },
                    },
                };

                let sa = unsafe {
                    siocgifhwaddr(fd.as_raw_fd(), &mut req as *mut _)?;
                    req.ifr_ifru.ifru_hwaddr.sa_data
                };

                MacAddress::try_from(sa.as_slice())?
            }
            _ => MacAddress::generate(),
        };

        Ok(Self {
            name,
            fd,
            idx,
            poll,
            tx,
            rx: Some(rx),
            mac,
        })
    }

    fn read_from_device(&mut self) -> AppResult<()> {
        let mut buf = [0u8; 1024];
        let sz = self.fd.read(&mut buf)?;
        tracing::trace!("[tap] read {sz} bytes");
        Ok(())
    }

    fn write_to_device(&mut self, pkt: Ipv4Packet) -> AppResult<()> {
        let iovs = [IoSlice::new(pkt.as_bytes())];
        let sz = self.fd.write_vectored(&iovs)?;

        tracing::trace!("[tap] wrote {sz} bytes");

        Ok(())
    }
}

impl Debug for TunTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tun({:02}:{})", self.idx, self.name)
    }
}

impl Wan for TunTap {
    fn as_wan_handle(&self) -> AppResult<Box<dyn WanHandle>> {
        let waker = Waker::new(self.poll.registry(), TOKEN_WRITE)?;
        self.poll.registry().register(
            &mut SourceFd(&self.fd.as_raw_fd()),
            TOKEN_READ,
            Interest::READABLE,
        )?;

        let handle = TunTapHandle {
            tx: self.tx.clone(),
            waker: Arc::new(waker),
        };

        Ok(Box::new(handle))
    }

    fn run(mut self: Box<Self>, _router: RouterHandle) -> AppResult<()> {
        let mut events = Events::with_capacity(MAX_EVENTS_CAPACITY);

        let rx = self
            .rx
            .take()
            .ok_or_else(|| Error::General(String::from("no receiver available, already used")))?;

        loop {
            self.poll.poll(&mut events, None)?;

            for event in &events {
                match event.token() {
                    TOKEN_READ => match self.read_from_device() {
                        Ok(_) => (),
                        Err(error) => {
                            tracing::warn!(?error, "[upstream] unable to read from tun device")
                        }
                    },
                    TOKEN_WRITE => {
                        for pkt in rx.drain() {
                            match self.write_to_device(pkt) {
                                Ok(()) => {
                                    tracing::trace!("[upstream] wrote ipv4 packet to tun device")
                                }
                                Err(error) => tracing::error!(
                                    ?error,
                                    "[upstream] unable to write to tun device"
                                ),
                            }
                        }
                    }
                    Token(token) => tracing::trace!(token, "[tap] unknown mio token"),
                }
            }
        }
    }
}

impl WanHandle for TunTapHandle {
    fn write(&self, pkt: Ipv4Packet) -> Result<(), crate::error::Error> {
        self.tx.send(pkt).ok();
        self.waker.wake().ok();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::TunTap;

    #[test]
    fn open_tap() {
        TunTap::create_tap("oathgate1".into()).expect("unable to open tap");
    }
}
