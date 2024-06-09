use std::{
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::thread::JoinHandleExt,
    },
    path::PathBuf,
    sync::Arc,
};

use clap::Parser;
use crossterm::{
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use events::EventHandler;
use mio::{
    unix::{pipe::Receiver, SourceFd},
    Events, Interest, Poll, Token,
};
use nix::{
    sys::{
        signal::{SigSet, Signal},
        signalfd::{SfdFlags, SignalFd},
        socket::{AddressFamily, SockFlag, VsockAddr},
    },
    unistd::Pid,
};
use parking_lot::RwLock;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use serde::{Deserialize, Serialize};
use tracing::Level;
use tui_term::{vt100, widget::PseudoTerminal};
use vm::VmHandle;

mod events;
mod vm;

type Error = Box<dyn std::error::Error + 'static>;
type VT100 = Arc<RwLock<TermEmulator>>;

#[derive(Parser)]
pub struct Opts {
    /// Path to configuration file
    config: PathBuf,

    /// Verbosity (-v, -vv, -vvv)
    #[clap(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MachineConfig {
    pub cpu: String,
    pub memory: String,
    pub kernel: PathBuf,
    pub disk: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    machine: MachineConfig,
}

pub struct TermEmulator {
    pty: vt100::Parser,
    rows: u16,
    cols: u16,
}

impl TermEmulator {
    pub fn new() -> Arc<RwLock<Self>> {
        Arc::new(RwLock::new(Self::default()))
    }

    pub fn set_rect(&mut self, r: Rect) {
        self.set_size(r.height, r.width);
    }

    pub fn set_size(&mut self, rows: u16, cols: u16) {
        if self.rows != rows && self.cols != cols {
            self.rows = rows - 2;
            self.cols = cols;
            self.pty.set_size(self.rows, self.cols);
            tracing::debug!(rows = self.rows, cols = self.cols, "updating term size");
        }
    }

    pub fn process(&mut self, buf: &[u8]) {
        self.pty.process(buf);
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.pty.screen()
    }

    /// Returns the size of the terminal area in (rows, cols)
    pub fn size(&self) -> (u16, u16) {
        (self.rows, self.cols)
    }
}

impl Default for TermEmulator {
    fn default() -> Self {
        let rows = 24;
        let cols = 80;
        let scrollback = 0; // infinite
        let pty = vt100::Parser::new(rows, cols, scrollback);
        Self { pty, rows, cols }
    }
}

fn bind_vhost() -> Result<OwnedFd, Error> {
    use nix::sys::socket::{
        bind, listen, socket, AddressFamily, Backlog, SockFlag, SockType, VsockAddr,
    };

    let addr = VsockAddr::new(2, 3715);
    let sfd = socket(
        AddressFamily::Vsock,
        SockType::Stream,
        SockFlag::SOCK_NONBLOCK,
        None,
    )?;
    bind(sfd.as_raw_fd(), &addr)?;
    listen(&sfd, Backlog::new(10)?)?;

    Ok(sfd)
}

fn run_vm(pty: VT100, mut handle: VmHandle, mut rx: Receiver) -> Result<(), Error> {
    const TOKEN_STDIN: Token = Token(0);
    const TOKEN_STDOUT: Token = Token(1);
    const TOKEN_STDERR: Token = Token(2);
    const TOKEN_SIGNAL: Token = Token(3);
    const TOKEN_HYPERVISOR: Token = Token(4);
    const TOKEN_PTY: Token = Token(5);

    let mut poller = Poll::new()?;

    let mut stdout = handle.stdout_receiver()?;
    let mut stderr = handle.stderr_receiver()?;

    stdout.set_nonblocking(true)?;
    stderr.set_nonblocking(true)?;

    let mut mask = SigSet::empty();
    mask.add(nix::sys::signal::SIGTERM);
    mask.thread_block()?;

    let sfd = SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK)?;

    // bind a vhost socket
    let vhost = bind_vhost()?;

    poller
        .registry()
        .register(&mut rx, TOKEN_STDIN, Interest::READABLE)?;
    poller
        .registry()
        .register(&mut stdout, TOKEN_STDOUT, Interest::READABLE)?;
    poller
        .registry()
        .register(&mut stderr, TOKEN_STDERR, Interest::READABLE)?;
    poller.registry().register(
        &mut SourceFd(&sfd.as_raw_fd()),
        TOKEN_SIGNAL,
        Interest::READABLE,
    )?;
    poller.registry().register(
        &mut SourceFd(&vhost.as_raw_fd()),
        TOKEN_HYPERVISOR,
        Interest::READABLE,
    )?;

    //let stdout_log = File::create("stdout.log")?;
    //let mut stdout_log = BufWriter::new(stdout_log);

    let mut buf = [0u8; 4096];
    let mut events = Events::with_capacity(10);
    'poll: loop {
        poller.poll(&mut events, None)?;

        for event in &events {
            match event.token() {
                TOKEN_STDIN => {
                    let sz = rx.read(&mut buf)?;
                    handle.write_pty(&buf[..sz])?;
                }
                TOKEN_STDOUT => stdout.try_io(|| {
                    let sz = nix::unistd::read(stdout.as_raw_fd(), &mut buf)?;
                    tracing::trace!("read {sz} bytes from stdout");
                    pty.write().process(&buf[..sz]);

                    Ok(())
                })?,
                TOKEN_STDERR => stderr.try_io(|| {
                    let sz = nix::unistd::read(stderr.as_raw_fd(), &mut buf)?;
                    let msg = String::from_utf8_lossy(&buf[..sz]);
                    tracing::warn!(%msg, "qemu stderr");

                    Ok(())
                })?,
                TOKEN_SIGNAL => match sfd.read_signal()? {
                    Some(sig) => match Signal::try_from(sig.ssi_signo as i32) {
                        Ok(Signal::SIGTERM) => break 'poll,
                        Ok(signal) => tracing::debug!(%signal, "caught unhandled signal"),
                        Err(error) => tracing::warn!(%error, "unknown signal number"),
                    },
                    None => (),
                },
                TOKEN_HYPERVISOR => {
                    let mut vmid = [0u8; 2];
                    let csock = nix::sys::socket::accept(vhost.as_raw_fd())?;
                    nix::unistd::read(csock, &mut vmid)?;
                    let vmid = u16::from_le_bytes(vmid);

                    tracing::debug!(%vmid, "vm starting, opening connection to vm tty");

                    let addr = VsockAddr::new(vmid.into(), 3715);
                    let sock = nix::sys::socket::socket(
                        AddressFamily::Vsock,
                        nix::sys::socket::SockType::Stream,
                        SockFlag::empty(),
                        None,
                    )?;
                    nix::sys::socket::connect(sock.as_raw_fd(), &addr)?;

                    poller.registry().register(
                        &mut SourceFd(&sock.as_raw_fd()),
                        TOKEN_PTY,
                        Interest::READABLE,
                    )?;

                    let (rows, cols) = pty.read().size();
                    handle.set_pty(sock);
                    handle.resize_pty(rows, cols)?;
                }
                TOKEN_PTY => {
                    let sz = handle.read_pty(&mut buf)?;
                    pty.write().process(&buf[..sz]);
                }
                Token(token) => tracing::debug!(%token, "unknown mio token"),
            }
        }
    }

    tracing::debug!("sending SIGTERM to vm");
    nix::sys::signal::kill(Pid::from_raw(handle.pid() as i32), Signal::SIGTERM)?;

    tracing::debug!("waiting for vm process to stop");
    drop(stdout);
    drop(stderr);
    handle.wait()?;

    Ok(())
}

fn run_tui<W: Write>(pty: VT100, mut stdin: W) -> Result<(), Error> {
    crossterm::terminal::enable_raw_mode()?;
    std::io::stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;

    terminal.clear()?;

    let mut events = EventHandler::default();
    let mut should_quit = false;
    while !should_quit {
        terminal.draw(|f| ui(f, &pty))?;
        should_quit = events.handle(&pty, &mut stdin)?;
    }

    crossterm::terminal::disable_raw_mode()?;
    std::io::stdout().execute(LeaveAlternateScreen)?;

    Ok(())
}

fn ui(frame: &mut Frame, term: &VT100) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(&[Constraint::Percentage(100), Constraint::Min(1)])
        .split(frame.size());

    term.write().set_rect(chunks[0]);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("[Running: VM]")
        .style(Style::default().add_modifier(Modifier::BOLD));

    let tr = term.read();
    let pty = PseudoTerminal::new(tr.screen()).block(block);
    frame.render_widget(pty, chunks[0]);

    let block = Block::default().borders(Borders::ALL);
    frame.render_widget(block, frame.size());

    let explanation = Paragraph::new("Press F4 to quit")
        .style(Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED))
        .alignment(Alignment::Center);

    frame.render_widget(explanation, chunks[1]);
}

fn main() -> Result<(), Error> {
    let opts = Opts::parse();

    let fd = File::create("output.log")?;
    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .with_writer(fd)
        .init();

    let fd = File::open(&opts.config)?;
    let cfg: Config = serde_yaml::from_reader(fd)?;

    let vm_handle = VmHandle::new("/tmp/oathgate.sock", cfg.machine)?;

    let (tx, rx) = mio::unix::pipe::new().unwrap();
    let parser = TermEmulator::new();

    let handle = std::thread::Builder::new()
        .name(String::from("qemu-vm"))
        .spawn({
            let parser = Arc::clone(&parser);
            move || {
                if let Err(error) = run_vm(parser, vm_handle, rx) {
                    tracing::error!(?error, "unable to run qemu vm");
                }
            }
        })?;

    run_tui(parser, tx)?;

    nix::sys::pthread::pthread_kill(handle.as_pthread_t(), Signal::SIGTERM)?;

    match handle.join() {
        Ok(_) => tracing::info!("vm stopped"),
        Err(error) => tracing::error!(?error, "unable to stop vm"),
    }

    Ok(())
}
