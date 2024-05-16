//! UDP upstream.  Forwards traffic to a specific UDP port

use std::{io::IoSlice, net::{SocketAddr, ToSocketAddrs, UdpSocket}, os::fd::{AsRawFd, RawFd}};

use nix::sys::socket::{sendmsg, MsgFlags, SockaddrIn, SockaddrIn6};

use crate::{error::AppResult, router::{protocols::Ipv4Header, RouterHandle}};

use super::UpstreamHandle;

pub struct UdpDevice{
    sock: UdpSocket,
    dests: Vec<SocketAddr>,
}

pub struct UdpDeviceHandle {
    sock: RawFd,
    dests: Vec<SocketAddr>,
}

impl UdpDevice {
    pub fn connect<A: ToSocketAddrs>(addrs: A) -> AppResult<Self> {
        let sock = UdpSocket::bind("0.0.0.0:0")?;
        let dests = addrs.to_socket_addrs()?.collect::<Vec<_>>();
        Ok(Self { sock, dests })
    }

    pub fn spawn(self, router: RouterHandle) -> AppResult<()> {
        let handle = UdpDeviceHandle {
            sock: self.sock.as_raw_fd(),
            dests: self.dests.clone(),
        };

        router.set_upstream(Box::new(handle));

        std::thread::Builder::new().name(String::from("upstream-udp")).spawn(move || {
            if let Err(error) = self.run(router) {
                tracing::error!(?error, "unable to run upstream udp device");
            }
        })?;

        Ok(())
    }

    fn run(self, router: RouterHandle) -> AppResult<()> {
        let mut buf = [0u8; 1600];
        loop {
            let (sz, peer) = self.sock.recv_from(&mut buf)?;
            tracing::debug!(?peer, "read {sz} bytes from peer");
            let pkt = buf[0..sz].to_vec();
            router.route(0, pkt);
        }
    }
}

impl UpstreamHandle for UdpDeviceHandle {
    fn write(&self, hdr: Ipv4Header, buf: Vec<u8>) -> AppResult<()> {
        let hdr = hdr.into_bytes();
        let iov = [IoSlice::new(&hdr), IoSlice::new(&buf)];

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
