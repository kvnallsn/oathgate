//! Fabrial client to connect to a fabrial tty server

use std::{
    fs::File,
    io::{self, ErrorKind, Write},
    os::fd::{AsRawFd, OwnedFd, RawFd},
    path::PathBuf,
};

use clap::Parser;
use crossterm::{
    event::{Event, KeyCode, KeyModifiers},
    terminal,
};
use fabrial::{Error, ErrorContext};
use mio::{unix::SourceFd, Events, Interest, Poll, Token};
use nix::{
    errno::Errno,
    sys::socket::{AddressFamily, MsgFlags, SockFlag, SockType, VsockAddr},
};
use tracing::Level;

macro_rules! encode {
    ($bytes:expr, $b0: expr) => {{
        $bytes[3] = $b0;
        1
    }};
    ($bytes:expr, $b0: expr, $b1:expr) => {{
        $bytes[3] = $b0;
        $bytes[4] = $b1;
        2
    }};
    ($bytes:expr, $b0: expr, $b1:expr, $b2:expr) => {{
        $bytes[3] = $b0;
        $bytes[4] = $b1;
        $bytes[5] = $b2;
        3
    }};
    ($bytes:expr, $b0: expr, $b1:expr, $b2:expr, $b3:expr) => {{
        $bytes[3] = $b0;
        $bytes[4] = $b1;
        $bytes[5] = $b2;
        $bytes[6] = $b3;
        4
    }};
    ($bytes:expr, $b0: expr, $b1:expr, $b2:expr, $b3:expr, $b4:expr) => {{
        $bytes[3] = $b0;
        $bytes[4] = $b1;
        $bytes[5] = $b2;
        $bytes[6] = $b3;
        $bytes[7] = $b4;
        5
    }};
}

#[derive(Debug, Parser)]
struct Opts {
    /// Virtual Machine Id (aka cid) to connect
    cid: u32,

    /// Port to connect on the vsock socket
    #[clap(short, long, default_value = "3715")]
    port: u32,

    /// Path to file on disk to store logs
    #[clap(short, long, default_value = "fabrial.log")]
    logfile: PathBuf,

    /// Verbosity of output (-v, -vv, -vvv)
    #[clap(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn connect(cid: u32, port: u32) -> io::Result<OwnedFd> {
    let addr = VsockAddr::new(cid, port);
    let sock = nix::sys::socket::socket(
        AddressFamily::Vsock,
        SockType::Stream,
        SockFlag::empty(),
        None,
    )?;
    nix::sys::socket::connect(sock.as_raw_fd(), &addr)?;
    Ok(sock)
}

fn run(sock: RawFd) -> Result<(), Error> {
    use nix::sys::socket;

    const TOKEN_SOCKET: Token = Token(0);
    const MAX_EVENTS: usize = 10;

    let mut poller = Poll::new()?;

    poller
        .registry()
        .register(&mut SourceFd(&sock), TOKEN_SOCKET, Interest::READABLE)?;

    let mut buf = [0u8; 2048];
    let mut events = Events::with_capacity(MAX_EVENTS);
    'poll: loop {
        match poller.poll(&mut events, None) {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == ErrorKind::Interrupted => Ok(()),
            Err(error) => Err(error).context("poll failed"),
        }?;

        for event in &events {
            match event.token() {
                TOKEN_SOCKET if event.is_readable() => 'socket: loop {
                    let sz = match socket::recv(sock, &mut buf, MsgFlags::MSG_DONTWAIT) {
                        Ok(0) => {
                            tracing::info!("read 0 bytes, quitting");
                            break 'poll;
                        }
                        Ok(sz) => {
                            tracing::trace!("read {sz} bytes from socket");
                            sz
                        }
                        Err(Errno::EWOULDBLOCK) => break 'socket,
                        Err(errno) => Err(errno).context("unable to receive data")?,
                    };

                    io::stdout()
                        .write(&buf[..sz])
                        .context("stdout write failed")?;

                    io::stdout()
                        .flush()
                        .context("unable to flush stdout stream")?;
                },
                TOKEN_SOCKET => {
                    tracing::warn!("got unknown event on vhost socket");
                    break 'poll;
                }
                Token(token) => tracing::debug!(%token, "unknown mio token"),
            }
        }
    }

    nix::sys::socket::shutdown(sock, socket::Shutdown::Both)
        .context("unable to shutdown socket")?;

    tracing::info!("client closed connection");
    Ok(())
}

fn resize_pty(sock: RawFd, rows: u16, cols: u16, bytes: &mut [u8]) -> Result<(), Error> {
    use nix::sys::socket;

    tracing::debug!("resizing terminal to {rows}x{cols}");

    bytes[0] = 0x02;
    bytes[1..3].copy_from_slice(&rows.to_le_bytes());
    bytes[3..5].copy_from_slice(&cols.to_le_bytes());

    socket::send(sock, &bytes[0..5], MsgFlags::MSG_DONTWAIT)?;
    Ok(())
}

fn run_ui(sock: RawFd) -> Result<(), Error> {
    use nix::sys::socket;
    const MODIFIERS_EMPTY: KeyModifiers = KeyModifiers::empty();
    const MODIFIERS_CTRL: KeyModifiers = KeyModifiers::CONTROL;
    const MODIFIERS_SHIFT: KeyModifiers = KeyModifiers::SHIFT;

    let mut bytes: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

    // get the terminal size
    if let Err(error) = terminal::size()
        .context("unable to get terminal size")
        .and_then(|(cols, rows)| resize_pty(sock, rows, cols, &mut bytes))
        .context("resize_pty error")
    {
        tracing::warn!(?error, "unable to resize terminal");
    }

    loop {
        let ev = crossterm::event::read()?;
        tracing::trace!("got event: {ev:?}");

        match ev {
            Event::Key(ev) => {
                let sz = match ev.modifiers {
                    MODIFIERS_EMPTY => match ev.code {
                        KeyCode::Char(ch) => ch.encode_utf8(&mut bytes[3..]).len(),
                        KeyCode::Backspace => encode!(bytes, 0x08),
                        KeyCode::Tab => encode!(bytes, 0x09),
                        KeyCode::Enter => encode!(bytes, 0x0A),
                        KeyCode::Up => encode!(bytes, 0x1B, 0x5B, 0x41),
                        KeyCode::Down => encode!(bytes, 0x1B, 0x5B, 0x42),
                        KeyCode::Right => encode!(bytes, 0x1B, 0x5B, 0x43),
                        KeyCode::Left => encode!(bytes, 0x1B, 0x5B, 0x44),
                        KeyCode::F(1) => encode!(bytes, 0x1B, 0x4F, 0x50),
                        KeyCode::F(2) => encode!(bytes, 0x1B, 0x4F, 0x51),
                        KeyCode::F(3) => encode!(bytes, 0x1B, 0x4F, 0x52),
                        KeyCode::F(4) => encode!(bytes, 0x1B, 0x4F, 0x53),
                        KeyCode::F(5) => encode!(bytes, 0x1B, 0x5B, 0x01, 0x05, 0x7E),
                        KeyCode::F(6) => encode!(bytes, 0x1B, 0x5B, 0x01, 0x07, 0x7E),
                        KeyCode::F(7) => encode!(bytes, 0x1B, 0x5B, 0x01, 0x08, 0x7E),
                        KeyCode::F(8) => encode!(bytes, 0x1B, 0x5B, 0x01, 0x09, 0x7E),
                        KeyCode::F(9) => encode!(bytes, 0x1B, 0x5B, 0x02, 0x00, 0x7E),
                        KeyCode::F(10) => encode!(bytes, 0x1B, 0x5B, 0x02, 0x01, 0x7E),
                        KeyCode::F(11) => encode!(bytes, 0x1B, 0x5B, 0x02, 0x03, 0x7E),
                        KeyCode::F(12) => encode!(bytes, 0x1B, 0x5B, 0x02, 0x04, 0x7E),
                        _ => 0,
                    },
                    MODIFIERS_CTRL => match ev.code {
                        KeyCode::Char('c') => encode!(bytes, 0x03),
                        KeyCode::Char('d') => encode!(bytes, 0x04),
                        KeyCode::Char('r') => encode!(bytes, 0x12),
                        KeyCode::Char('u') => encode!(bytes, 0x15),
                        KeyCode::Char('z') => encode!(bytes, 0x1A),
                        _ => 0,
                    },
                    MODIFIERS_SHIFT => match ev.code {
                        KeyCode::Char(ch) => ch.encode_utf8(&mut bytes[3..]).len(),
                        _ => 0,
                    },
                    _ => 0,
                };

                if sz > 0 {
                    let szb = sz.to_le_bytes();
                    bytes[0] = 0x01;
                    bytes[1] = szb[0];
                    bytes[2] = szb[1];

                    let sz = socket::send(sock, &bytes[0..(sz + 3)], MsgFlags::MSG_DONTWAIT)?;
                    tracing::trace!("wrote {sz} bytes to vhost socket");
                }
            }
            Event::Resize(cols, rows) => resize_pty(sock, rows, cols, &mut bytes)?,
            _ => (),
        }
    }
}

fn main() {
    let opts = Opts::parse();

    let level = match opts.verbose {
        0 => Level::WARN,
        1 => Level::INFO,
        2 => Level::DEBUG,
        _ => Level::TRACE,
    };

    let logfile = File::create(&opts.logfile).expect("unable to create log file");
    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(level)
        .with_writer(logfile)
        .pretty()
        .init();

    let sock = match connect(opts.cid, opts.port).context("unable to connect to socket") {
        Ok(sock) => sock,
        Err(error) => {
            let context = error.context().unwrap_or("None");
            tracing::error!(error = %error.source(), %context, "socket error");
            std::process::exit(-2);
        }
    };

    let sfd = sock.as_raw_fd();
    std::thread::Builder::new()
        .name(String::from("fabrial-io"))
        .spawn(move || {
            let res = terminal::enable_raw_mode()
                .context("unable to enable terminal raw mode")
                .and_then(|_| run_ui(sfd))
                .context("failed to run ui event loop");

            if let Err(error) = res {
                let context = error.context().unwrap_or("None");
                tracing::error!(error = %error.source(), %context, "ui thread died");
            }

            terminal::disable_raw_mode().ok();
        })
        .ok();

    match run(sfd) {
        Ok(_) => {
            tracing::info!("connection closed");
        }
        Err(error) => {
            let context = error.context().unwrap_or("None");
            tracing::error!(error = %error.source(), %context, "fabrial-client failed");
        }
    }
}
