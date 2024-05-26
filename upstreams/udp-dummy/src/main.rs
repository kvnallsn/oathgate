use std::net::UdpSocket;

use anyhow::Result;
use clap::Parser;
use oathgate_net::{
    protocols::{icmp::DestinationUnreachableCode, IcmpPacket},
    Ipv4Header,
};
use tracing::Level;

const MTU_SZ: usize = 1600;

#[derive(Parser)]
struct Opts {
    /// Port for UDP socket to listen
    #[clap(short, long, default_value_t = 9870)]
    port: u16,
}

fn main() -> Result<()> {
    let opts = Opts::parse();

    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .init();

    let sock = UdpSocket::bind(format!("0.0.0.0:{}", opts.port))?;
    tracing::info!(port = opts.port, "bound udp socket");

    let mut buf = [0u8; MTU_SZ];
    while let Ok((sz, peer)) = sock.recv_from(&mut buf) {
        tracing::debug!(?peer, "read {sz} bytes from socket");

        let hdr = Ipv4Header::extract_from_slice(&buf)?;
        tracing::debug!("ipv4 header: {:02x?}", hdr);

        let icmp = IcmpPacket::destination_unreachable(
            DestinationUnreachableCode::NetworkUnreachable,
            &hdr,
            &buf[20..],
        );

        let sz = icmp.as_bytes(&mut buf[20..]);
        let hdr = hdr.gen_reply(&buf[20..(20 + sz)]);
        tracing::debug!(?hdr, "response header");
        hdr.as_bytes(&mut buf);

        tracing::debug!("header: {:02x?}", &buf[..20]);

        let sz = sock.send_to(&buf[..(20 + sz)], peer)?;
        tracing::debug!("wrote {sz} bytes to socket");
    }

    Ok(())
}
