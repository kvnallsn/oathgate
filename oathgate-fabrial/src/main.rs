use std::os::fd::AsRawFd;

use clap::Parser;
use nix::sys::socket::{bind, socket, AddressFamily, SockFlag, SockType, VsockAddr};
use oathgate_net::types::MacAddress;

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Parser)]
struct Opts {
    /// Name of the interface used to generate cid
    #[clap(short, long, default_value = "eth0")]
    interface: String,

    /// Port to bind on vsock socket
    #[clap(short, long, default_value = "3715")]
    port: u32,

    /// Verbosity of output (-v, -vv, -vvv)
    #[clap(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() {
    let opts = Opts::parse();

    let level = match opts.verbose {
        0 => tracing::Level::WARN,
        1 => tracing::Level::INFO,
        2 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };

    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(level)
        .init();

    let mac = MacAddress::from_interface(&opts.interface)
        .expect(&format!("unable to get mac from iface {}", &opts.interface));

    let bytes = mac.as_bytes();
    let cid = u32::from_be_bytes([0x00, 0x00, bytes[4], bytes[5]]);

    tracing::info!("derived cid from mac: {mac} -> {cid:04x}");

    let addr = VsockAddr::new(cid, opts.port);
    let sock = socket(
        AddressFamily::Vsock,
        SockType::Stream,
        SockFlag::SOCK_NONBLOCK,
        None,
    )
    .expect("unable to create socket");

    bind(sock.as_raw_fd(), &addr).expect("unable to bind socket");

    println!("Hello, world!");
}
