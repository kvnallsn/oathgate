use std::{
    collections::HashMap,
    fs::File,
    io,
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
use mio::{unix::SourceFd, Events, Interest, Poll, Token};
use nix::{
    sys::{
        signal::{SigSet, Signal},
        signalfd::{SfdFlags, SignalFd},
    },
    unistd::Pid,
};
use parking_lot::RwLock;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use serde::{Deserialize, Serialize};
use tracing::Level;
use tui_term::widget::PseudoTerminal;
use vm::VmHandle;

use crate::pty::{FabrialPty, OathgatePty, PipePty};

mod events;
mod pty;
mod vm;

type Error = Box<dyn std::error::Error + 'static>;
type ArcTerminalMap = Arc<RwLock<TerminalMap>>;

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

pub struct TerminalMap {
    terminals: HashMap<usize, Box<dyn OathgatePty>>,
    winsz: (u16, u16),
    active: Option<usize>,
}

impl TerminalMap {
    /// Adds a new pty to the terminal map
    ///
    /// ### Arguments
    /// * `token` - Unique id used to identify this pty
    /// * `pty` - Psuedo-terminal linked to virtual machine
    pub fn insert(&mut self, token: usize, pty: Box<dyn OathgatePty>) {
        let (rows, cols) = self.get_size();
        if let Err(error) = pty.resize_pty(rows, cols) {
            tracing::warn!(?error, "unable to resize pty");
        }

        tracing::debug!("created new pty with id {token}");
        self.terminals.insert(token, pty);
    }

    /// Returns a reference to the terminal with the corresponding unique id (token),
    /// or None if one does not exist
    ///
    /// ### Arguments
    /// * `token` - Unique id representing a VM
    pub fn get(&self, token: usize) -> Option<&Box<dyn OathgatePty>> {
        self.terminals.get(&token)
    }

    /// Returns a reference to the active pty if one is set, or `None` if there is
    /// no active pty
    ///
    /// The active pty represents the pty currently being displayed by a TUI
    pub fn get_active(&self) -> Option<&Box<dyn OathgatePty>> {
        self.active.and_then(|id| self.get(id))
    }

    /// Sets the pty with the corresponding id (token) as the active pty
    ///
    /// ### Arguments
    /// * `token` - Unique id of the pty to set as active
    pub fn set_active(&mut self, token: usize) {
        self.active = Some(token);
    }

    /// Helper function to write to the active pty.  If no pty is set as active,
    /// the data is discarded (not written or queued).
    ///
    /// ### Arguments
    /// * `data` - Data to write to the active pty
    pub fn write_to_pty(&self, data: &[u8]) -> io::Result<()> {
        match self.get_active() {
            Some(term) => term.write_pty(data),
            None => Ok(()),
        }
    }

    /// Returns an iterator over all ptys currently stored in this terminal map
    pub fn all(&self) -> impl Iterator<Item = &Box<dyn OathgatePty>> {
        self.terminals.values()
    }

    /// Sets the size to use when creating new ptys
    ///
    /// ### Arguments
    /// * `rows` - Number of rows to set in the pty
    /// * `cols` - Number of columns to set in the pty
    pub fn set_size(&mut self, rows: u16, cols: u16) {
        let (old_rows, old_cols) = self.winsz;
        if rows != old_rows || cols != old_cols {
            tracing::debug!(
                "setting default pty size to {rows}x{cols} (was: {old_rows}x{old_cols})"
            );
            self.winsz = (rows, cols);

            // update all terminals, as applicable
            for term in self.all() {
                if let Err(error) = term.resize_pty(rows, cols) {
                    tracing::warn!(?error, "unable to resize terminal");
                }
            }
        }
    }

    /// Returns the current size used to create ptys
    pub fn get_size(&self) -> (u16, u16) {
        self.winsz
    }
}

impl Default for TerminalMap {
    fn default() -> Self {
        Self {
            terminals: HashMap::new(),
            winsz: (24, 80),
            active: None,
        }
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

fn run_vm(terminals: ArcTerminalMap, mut handle: VmHandle) -> Result<(), Error> {
    const TOKEN_STDERR: Token = Token(2);
    const TOKEN_SIGNAL: Token = Token(3);
    const TOKEN_HYPERVISOR: Token = Token(4);

    let mut poller = Poll::new()?;
    let mut poller_next_id = 10;

    let mut stderr = handle.stderr()?;
    stderr.set_nonblocking(true)?;

    let mut mask = SigSet::empty();
    mask.add(nix::sys::signal::SIGTERM);
    mask.thread_block()?;

    let sfd = SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK)?;

    // bind a vhost socket
    let vhost = bind_vhost()?;

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

    // register terminals
    let mut stdio_term = PipePty::new(handle.child_mut())?;
    poller
        .registry()
        .register(&mut stdio_term, Token(1), Interest::READABLE)?;
    terminals.write().insert(1, Box::new(stdio_term));
    terminals.write().set_active(1);

    //let stdout_log = File::create("stdout.log")?;
    //let mut stdout_log = BufWriter::new(stdout_log);

    let mut buf = [0u8; 4096];
    let mut events = Events::with_capacity(10);
    'poll: loop {
        poller.poll(&mut events, None)?;

        for event in &events {
            match event.token() {
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

                    tracing::debug!(%vmid, "vm starting, opening fabrial connection to vm");

                    match FabrialPty::connect(vmid as u32, 3715) {
                        Ok(mut fpty) => {
                            tracing::debug!("fabrial connected, switching ptys");
                            poller.registry().register(
                                &mut fpty,
                                Token(poller_next_id),
                                Interest::READABLE,
                            )?;
                            terminals.write().insert(poller_next_id, Box::new(fpty));
                            terminals.write().set_active(poller_next_id);
                            poller_next_id = poller_next_id + 1;
                        }
                        Err(error) => tracing::warn!(
                            ?error,
                            "unable to connect to fabrial, is the service running?"
                        ),
                    }
                }
                Token(token) => match terminals.read().get(token) {
                    None => tracing::debug!(token, "unknown mio token"),
                    Some(term) => {
                        term.read_pty(&mut buf)?;
                    }
                },
            }
        }
    }

    tracing::debug!("sending SIGTERM to vm");
    nix::sys::signal::kill(Pid::from_raw(handle.pid() as i32), Signal::SIGTERM)?;

    tracing::debug!("waiting for vm process to stop");
    drop(stderr);
    handle.wait()?;

    Ok(())
}

fn run_tui(terminals: ArcTerminalMap) -> Result<(), Error> {
    crossterm::terminal::enable_raw_mode()?;
    std::io::stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;

    terminal.clear()?;

    let mut events = EventHandler::default();
    let mut should_quit = false;
    while !should_quit {
        terminal.draw(|f| ui(f, &terminals))?;
        should_quit = events.handle(&terminals)?;
    }

    crossterm::terminal::disable_raw_mode()?;
    std::io::stdout().execute(LeaveAlternateScreen)?;

    Ok(())
}

fn ui(frame: &mut Frame, terminals: &ArcTerminalMap) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(&[Constraint::Percentage(100), Constraint::Min(1)])
        .split(frame.size());

    terminals
        .write()
        .set_size(chunks[0].height - 2, chunks[0].width);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("[Running: VM]")
        .style(Style::default().add_modifier(Modifier::BOLD));

    let tr = terminals.read();
    match tr.get_active() {
        Some(term) => {
            let pty = term.pty();
            let pty = pty.read();
            let pty = PseudoTerminal::new(pty.screen()).block(block);
            frame.render_widget(pty, chunks[0]);
        }
        None => (),
    }

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

    let terminals = Arc::new(RwLock::new(TerminalMap::default()));

    let vm_handle = VmHandle::new("/tmp/oathgate.sock", cfg.machine)?;

    let handle = std::thread::Builder::new()
        .name(String::from("qemu-vm"))
        .spawn({
            let terminals = Arc::clone(&terminals);
            move || {
                if let Err(error) = run_vm(terminals, vm_handle) {
                    tracing::error!(?error, "unable to run qemu vm");
                }
            }
        })?;

    run_tui(terminals)?;

    nix::sys::pthread::pthread_kill(handle.as_pthread_t(), Signal::SIGTERM)?;

    match handle.join() {
        Ok(_) => tracing::info!("vm stopped"),
        Err(error) => tracing::error!(?error, "unable to stop vm"),
    }

    Ok(())
}
