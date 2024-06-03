//! Various WAN providers

mod tap;
mod udp;
mod wireguard;

use oathgate_net::Ipv4Packet;

pub use self::{tap::TunTap, udp::UdpDevice, wireguard::{WgConfig, WgDevice}};

use super::{RouterError, RouterHandle};

pub trait Wan: Send + Sync
where
    Self: 'static,
{
    fn as_wan_handle(&self) -> Result<Box<dyn WanHandle>, RouterError>;

    fn run(self: Box<Self>, router: RouterHandle) -> Result<(), RouterError>;

    fn spawn(self: Box<Self>, router: RouterHandle) -> Result<Box<dyn WanHandle>, RouterError> {
        let handle = self.as_wan_handle()?;

        std::thread::Builder::new()
            .name(String::from("wan-thread"))
            .spawn(move || match self.run(router) {
                Ok(_) => tracing::trace!("wan thread exited successfully"),
                Err(error) => tracing::warn!(?error, "unable to run wan thread"),
            })?;

        Ok(handle)
    }
}

pub trait WanHandle: Send + Sync {
    /// Writes a packet to the upstream device
    fn write(&self, pkt: Ipv4Packet) -> Result<(), RouterError>;
}
