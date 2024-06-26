//! UDP upstream.  Forwards traffic to a specific UDP port

use std::{
    io::{self, IoSlice},
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    os::fd::{AsRawFd, RawFd},
};

use nix::sys::socket::{sendmsg, MsgFlags, SockaddrIn, SockaddrIn6};
use oathgate_net::Ipv4Packet;

use crate::net::{router::RouterHandle, NetworkError};

use super::{Wan, WanHandle};

pub struct UdpDevice {
    sock: UdpSocket,
    dests: Vec<SocketAddr>,
}

pub struct UdpDeviceHandle {
    sock: RawFd,
    dests: Vec<SocketAddr>,
}

impl UdpDevice {
    pub fn connect<A: ToSocketAddrs>(addrs: A) -> io::Result<Self> {
        let sock = UdpSocket::bind("0.0.0.0:0")?;
        let dests = addrs.to_socket_addrs()?.collect::<Vec<_>>();
        Ok(Self { sock, dests })
    }
}

impl Wan for UdpDevice
where
    Self: Sized,
{
    fn as_wan_handle(&self) -> Result<Box<dyn WanHandle>, NetworkError> {
        let handle = UdpDeviceHandle {
            sock: self.sock.as_raw_fd(),
            dests: self.dests.clone(),
        };

        Ok(Box::new(handle))
    }

    fn run(self: Box<Self>, router: RouterHandle) -> Result<(), NetworkError> {
        let mut buf = [0u8; 1600];
        loop {
            let (sz, peer) = self.sock.recv_from(&mut buf)?;
            tracing::trace!(?peer, "read {sz} bytes from peer: {:02x?}", &buf[..20],);
            let pkt = buf[0..sz].to_vec();
            match pkt[0] >> 4 {
                4 => {
                    let pkt = Ipv4Packet::parse(pkt)?;
                    router.route_ipv4(pkt)
                }
                6 => router.route_ipv6(pkt),
                version => tracing::warn!(version, "unknown ip version / malformed packet"),
            }
        }
    }
}

impl WanHandle for UdpDeviceHandle {
    fn write(&self, pkt: Ipv4Packet) -> Result<(), NetworkError> {
        let iov = [IoSlice::new(&pkt.as_bytes())];

        for dest in &self.dests {
            match dest {
                SocketAddr::V4(addr) => {
                    let addr = SockaddrIn::from(*addr);
                    sendmsg(self.sock, &iov, &[], MsgFlags::empty(), Some(&addr))?;
                }
                SocketAddr::V6(addr) => {
                    let addr = SockaddrIn6::from(*addr);
                    sendmsg(self.sock, &iov, &[], MsgFlags::empty(), Some(&addr))?;
                }
            }
        }
        Ok(())
    }
}
