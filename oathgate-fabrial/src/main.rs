mod tty;

use std::os::fd::{AsRawFd, OwnedFd};

use clap::Parser;
use mio::{unix::SourceFd, Events, Interest, Poll, Token};
use nix::sys::socket::{
    accept, bind, listen, socket, AddressFamily, Backlog, SockFlag, SockType, SockaddrIn, VsockAddr,
};
use oathgate_net::types::MacAddress;

use crate::tty::SockTTY;

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Parser)]
struct Opts {
    /// Type of shell to run
    #[clap(short, long, default_value = "/bin/bash")]
    command: String,

    /// Name of the interface used to generate cid
    #[clap(short, long, default_value = "eth0")]
    interface: String,

    /// Port to bind on vsock socket
    #[clap(short, long, default_value = "3715")]
    port: u32,

    /// True to use a TCP socket instead of a vsock
    #[clap(short, long)]
    tcp: bool,

    /// Verbosity of output (-v, -vv, -vvv)
    #[clap(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn run(opts: Opts) -> Result<(), Error> {
    const MAX_BACKLOG: i32 = 10;

    let sock = match opts.tcp {
        true => {
            let addr = SockaddrIn::new(127, 0, 0, 1, opts.port as u16);
            let sock = socket(
                AddressFamily::Inet,
                SockType::Stream,
                SockFlag::SOCK_NONBLOCK,
                None,
            )?;

            bind(sock.as_raw_fd(), &addr)?;
            tracing::info!("bound tcp port 127.0.0.1:{}", opts.port);

            sock
        }
        false => {
            let mac = MacAddress::from_interface(&opts.interface)?;

            let bytes = mac.as_bytes();
            let cid = u32::from_be_bytes([0x00, 0x00, bytes[4], bytes[5]]);

            tracing::info!("derived cid from mac: {mac} -> {cid:04x}");

            let addr = VsockAddr::new(cid, opts.port);
            let sock = socket(
                AddressFamily::Vsock,
                SockType::Stream,
                SockFlag::SOCK_NONBLOCK,
                None,
            )?;

            bind(sock.as_raw_fd(), &addr)?;
            tracing::info!("bound vsock cid={}, port={}", cid, opts.port);

            sock
        }
    };

    listen(&sock, Backlog::new(MAX_BACKLOG)?)?;
    poll(sock, &opts.command)?;

    Ok(())
}

fn poll(vsock: OwnedFd, cmd: &str) -> Result<(), Error> {
    const TOKEN_VSOCK: Token = Token(0);
    const MAX_EVENTS: usize = 10;

    let mut poller = Poll::new()?;

    poller.registry().register(
        &mut SourceFd(&vsock.as_raw_fd()),
        TOKEN_VSOCK,
        Interest::READABLE,
    )?;

    let mut events = Events::with_capacity(MAX_EVENTS);
    while let Ok(_) = poller.poll(&mut events, None) {
        for event in &events {
            match event.token() {
                TOKEN_VSOCK => {
                    let csock = accept(vsock.as_raw_fd())?;
                    SockTTY::spawn(csock, cmd)?;
                }
                Token(token) => tracing::debug!(token, "unknown mio token"),
            }
        }
    }

    Ok(())
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

    match run(opts) {
        Ok(_) => tracing::info!("quitting"),
        Err(error) => tracing::error!(?error, "unable to run fabrial"),
    }
}
