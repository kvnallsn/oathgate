mod device;
mod error;
mod queue;
mod types;

use device::TapDevice;
use tracing::Level;

fn main() {
    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .init();

    if let Err(error) = TapDevice::new(1).and_then(|mut dev| dev.run("/tmp/oathgate.sock")) {
        tracing::error!(?error, "unable to run");
    }
}
