mod error;
mod tty;

use std::{
    fs::File,
    io,
    net::Ipv4Addr,
    os::fd::{AsFd, AsRawFd, OwnedFd},
    path::PathBuf,
    time::Duration,
};

use clap::Parser;
use mio::{unix::SourceFd, Events, Interest, Poll, Token};
use nix::{
    libc::VMADDR_CID_ANY,
    sys::{
        socket::{
            accept, bind, connect, listen, socket, AddressFamily, Backlog, MsgFlags, SockFlag,
            SockType, SockaddrIn, SockaddrLike, VsockAddr,
        },
        timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags},
    },
};
use oathgate_net::types::MacAddress;
use tracing_subscriber::fmt::writer::{BoxMakeWriter, Tee};

pub use self::{error::Error, tty::SockTTY};

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

    /// Path to log file location
    #[clap(short, long, default_value = "/var/log/fabrial.log")]
    logfile: PathBuf,
}

#[allow(dead_code)]
struct Host {
    family: AddressFamily,
    addr: Box<dyn SockaddrLike>,
}

#[allow(dead_code)]
impl Host {
    pub fn new_tcp<A: Into<Ipv4Addr>>(addr: A, port: u16) -> Self {
        let octets = addr.into().octets();
        let addr = SockaddrIn::new(octets[0], octets[1], octets[2], octets[3], port);
        let family = AddressFamily::Inet;
        Self {
            family,
            addr: Box::new(addr),
        }
    }

    pub fn new_vsock(cid: u32, port: u32) -> Self {
        let addr = VsockAddr::new(cid, port);
        let family = AddressFamily::Vsock;
        Self {
            family,
            addr: Box::new(addr),
        }
    }

    pub fn connect(&self) -> io::Result<OwnedFd> {
        let sock = socket(self.family, SockType::Stream, SockFlag::SOCK_NONBLOCK, None)?;
        connect(sock.as_raw_fd(), self.addr.as_ref())?;
        Ok(sock)
    }

    pub fn notify_started(&self, id: u32) -> io::Result<()> {
        use nix::sys::socket;

        let sock = self.connect()?;
        socket::send(sock.as_raw_fd(), &id.to_le_bytes(), MsgFlags::empty())?;

        Ok(())
    }
}

fn run(opts: Opts) -> Result<(), Error> {
    const MAX_BACKLOG: i32 = 10;

    let mac = MacAddress::from_interface(&opts.interface)?;

    let bytes = mac.as_bytes();
    let cid = u32::from_be_bytes([0x00, 0x00, bytes[4], bytes[5]]);
    tracing::info!("derived cid from mac: {mac} -> {cid:04x}");

    let (sock, host) = match opts.tcp {
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

            let host = Host::new_tcp([127, 0, 0, 1], 3714);
            (sock, host)
        }
        false => {
            let addr = VsockAddr::new(VMADDR_CID_ANY, opts.port);
            let sock = socket(
                AddressFamily::Vsock,
                SockType::Stream,
                SockFlag::SOCK_NONBLOCK,
                None,
            )?;

            bind(sock.as_raw_fd(), &addr)?;
            tracing::info!("bound vsock cid={}, port={}", cid, opts.port);

            // the cid and port are the same (for now)
            let host = Host::new_vsock(cid, cid);
            (sock, host)
        }
    };

    listen(&sock, Backlog::new(MAX_BACKLOG)?)?;

    poll(sock, &opts.command, host, cid)?;

    Ok(())
}

fn poll(vsock: OwnedFd, cmd: &str, host: Host, id: u32) -> Result<(), Error> {
    const TOKEN_VSOCK: Token = Token(0);
    const TOKEN_HEALTH_TIMER: Token = Token(1);

    const MAX_EVENTS: usize = 10;

    let mut poller = Poll::new()?;
    let tfd = TimerFd::new(
        ClockId::CLOCK_MONOTONIC,
        TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC,
    )?;

    poller.registry().register(
        &mut SourceFd(&vsock.as_raw_fd()),
        TOKEN_VSOCK,
        Interest::READABLE,
    )?;

    poller.registry().register(
        &mut SourceFd(&tfd.as_fd().as_raw_fd()),
        TOKEN_HEALTH_TIMER,
        Interest::READABLE,
    )?;

    tfd.set(
        Expiration::OneShot(Duration::from_secs(5).into()),
        TimerSetTimeFlags::empty(),
    )?;

    let mut events = Events::with_capacity(MAX_EVENTS);
    while let Ok(_) = poller.poll(&mut events, None) {
        for event in &events {
            match event.token() {
                TOKEN_HEALTH_TIMER => {
                    // TODO: Health updates?
                    //if let Err(error) = host.notify_started(id) {
                    //    tracing::warn!(?error, "unable to notify host of vm start");
                    //11}
                }
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

    let logwriter = match File::options()
        .write(true)
        .create(true)
        .append(true)
        .open(&opts.logfile)
    {
        Ok(fd) => {
            tracing::info!(file = %opts.logfile.display(), "file logging enabled");
            BoxMakeWriter::new(Tee::new(std::io::stdout, fd))
        }
        Err(error) => {
            tracing::warn!(?error, "unable to create log file, only logging to stdout");
            BoxMakeWriter::new(std::io::stdout)
        }
    };

    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(level)
        .with_writer(logwriter)
        .init();

    match run(opts) {
        Ok(_) => tracing::info!("quitting"),
        Err(error) => tracing::error!(?error, "unable to run fabrial"),
    }
}
