//! Various WAN providers

mod tap;
mod udp;
mod wireguard;

use oathgate_net::Ipv4Packet;

use crate::error::{AppResult, Error};

pub use self::{tap::TunTap, udp::UdpDevice, wireguard::WgDevice};

use super::RouterHandle;

pub trait Wan: Send + Sync
where
    Self: 'static,
{
    fn as_wan_handle(&self) -> AppResult<Box<dyn WanHandle>>;

    fn run(self: Box<Self>, router: RouterHandle) -> AppResult<()>;

    fn spawn(self: Box<Self>, router: RouterHandle) -> AppResult<Box<dyn WanHandle>> {
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
    fn write(&self, pkt: Ipv4Packet) -> Result<(), Error>;
}
