mod config;
mod error;
mod net;

use std::{io::Read, net::SocketAddr, path::PathBuf};

use clap::Parser;
use config::Config;
use mio::{
    net::{TcpListener, TcpStream},
    Events, Interest, Poll, Token,
};
use oathgate_vhost::{DeviceOpts, VHostSocket};
use tracing::Level;

use crate::{
    config::WanConfig,
    error::Error,
    net::{
        router::{
            handler::IcmpHandler,
            Router,
        },
        switch::VirtioSwitch,
        wan::{TunTap, UdpDevice, Wan, WgDevice},
    }
};

#[derive(Parser)]
pub(crate) struct Opts {
    /// Path to configuration file
    pub config: PathBuf,

    /// Path to the unix socket to communicate with qemu's vhost-user driver
    #[arg(short, long, default_value = "/tmp/oathgate.sock")]
    pub socket: PathBuf,

    /// Path to pcap file, or blank to not capture pcap
    #[arg(short, long)]
    pub pcap: Option<PathBuf>,

    /// Control the level of output to stdout (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

fn parse_wan(cfg: WanConfig) -> Result<Option<Box<dyn Wan>>, Error> {
    match cfg {
        WanConfig::Tap(opts) => {
            let wan = TunTap::create_tap(opts.device)?;
            Ok(Some(Box::new(wan)))
        }
        WanConfig::Udp(opts) => {
            let wan = UdpDevice::connect(opts.endpoint)?;
            Ok(Some(Box::new(wan)))
        }
        WanConfig::Wireguard(opts) => {
            let wan = WgDevice::create(opts)?;
            Ok(Some(Box::new(wan)))
        }
    }
}

fn run(opts: Opts, cfg: Config) -> Result<(), Error> {
    const TOKEN_VHOST: Token = Token(0);
    const TOKEN_TELNET: Token = Token(1);

    let addr: SocketAddr = "127.0.0.1:3716".parse().unwrap();
    let mut telnet = TcpListener::bind(addr)?;

    let mut socket = VHostSocket::new(&opts.socket)?;
    let switch = VirtioSwitch::new(opts.pcap)?;

    // spawn the default route / upstream
    let wan = parse_wan(cfg.wan)?;

    // spawn thread to receive messages/packets
    let _router = Router::builder()
        .wan(wan)
        .register_proto_handler(IcmpHandler::default())
        .spawn(cfg.router.ipv4, switch.clone())?;

    let mut poller = Poll::new()?;
    poller
        .registry()
        .register(&mut socket, TOKEN_VHOST, Interest::READABLE)?;

    poller
        .registry()
        .register(&mut telnet, TOKEN_TELNET, Interest::READABLE)?;

    let mut buf = [0u8; 1024];
    let mut events = Events::with_capacity(10);
    loop {
        poller.poll(&mut events, None)?;

        for event in &events {
            match event.token() {
                TOKEN_VHOST => {
                    if let Err(error) =
                        socket.accept_and_spawn(DeviceOpts::default(), switch.clone())
                    {
                        tracing::error!(?error, "unable to accet connection");
                    }
                }
                TOKEN_TELNET => match telnet.accept() {
                    Ok((client, peer)) => {
                        tracing::info!(%peer, "accepted tcp connection");
                        match handle_tcp_client(client, &mut buf) {
                            Ok(_) => tracing::debug!("registered tcp client"),
                            Err(error) => tracing::warn!(?error, "unable to register tcp client"),
                        }
                    }
                    Err(error) => tracing::warn!(?error, "unable to accept tcp connection"),
                },
                Token(token) => tracing::debug!(%token, "[main] unknown mio token"),
            }
        }
    }
}

fn handle_tcp_client(mut strm: TcpStream, buf: &mut [u8]) -> Result<(), oathgate_vhost::Error> {
    const TOKEN_STRM: Token = Token(0);

    let mut poller = Poll::new()?;
    poller
        .registry()
        .register(&mut strm, TOKEN_STRM, Interest::READABLE)?;

    let mut events = Events::with_capacity(1);
    poller.poll(&mut events, None)?;

    // only one event so it should be ready now...

    // first message should be a command and port
    //
    // known values:
    // - `TERM <port>`
    let sz = strm.read(buf)?;
    let msg = buf[..sz].to_vec();
    let msg = String::from_utf8(msg).unwrap();
    let parts = msg.split(" ").collect::<Vec<_>>();
    let cmd = parts[0].trim();
    let port: u16 = parts[1].trim().parse().unwrap();

    match cmd {
        "TERM" => tracing::debug!("connecting to terminal port {port}"),
        _ => tracing::debug!("unknown tcp command: {cmd}"),
    }

    Ok(())
}

fn main() {
    let opts = Opts::parse();

    let level = match opts.verbose {
        0 => Level::WARN,
        1 => Level::INFO,
        2 => Level::DEBUG,
        _ => Level::TRACE,
    };

    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(level)
        .init();

    let cfg = Config::load(&opts.config).unwrap();
    tracing::debug!(?cfg, "configuration");

    if let Err(error) = run(opts, cfg) {
        tracing::error!(?error, "unable to run oathgate");
    }
}
