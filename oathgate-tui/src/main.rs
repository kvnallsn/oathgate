use std::{
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::thread::JoinHandleExt,
    },
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use mio::{
    unix::{pipe::Receiver, SourceFd},
    Events, Interest, Poll, Token,
};
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
use tui_term::{vt100, widget::PseudoTerminal};
use vm::VmHandle;

mod vm;

type Error = Box<dyn std::error::Error + 'static>;
type VT100 = Arc<RwLock<vt100::Parser>>;

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

fn run_vm(
    pty: Arc<RwLock<vt100::Parser>>,
    mut handle: VmHandle,
    mut rx: Receiver,
) -> Result<(), Error> {
    const TOKEN_STDIN: Token = Token(0);
    const TOKEN_STDOUT: Token = Token(1);
    const TOKEN_STDERR: Token = Token(2);
    const TOKEN_SIGNAL: Token = Token(3);

    let mut poller = Poll::new()?;

    let mut stdout = handle.stdout_receiver()?;
    let mut stderr = handle.stderr_receiver()?;

    stdout.set_nonblocking(true)?;
    stderr.set_nonblocking(true)?;

    let mut mask = SigSet::empty();
    mask.add(nix::sys::signal::SIGTERM);
    mask.thread_block()?;

    let mut sfd = SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK)?;

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

    let mut should_quit = false;
    while !should_quit {
        terminal.draw(|f| ui(f, pty.read().screen()))?;
        should_quit = handle_events(&pty, &mut stdin)?;
    }

    crossterm::terminal::disable_raw_mode()?;
    std::io::stdout().execute(LeaveAlternateScreen)?;

    Ok(())
}

fn handle_events<W: Write>(pty: &VT100, stdin: &mut W) -> io::Result<bool> {
    if event::poll(Duration::from_millis(50))? {
        match event::read()? {
            Event::Key(key) => match key.kind {
                KeyEventKind::Press => {
                    return handle_key_press(key, stdin);
                }
                KeyEventKind::Repeat => (),
                KeyEventKind::Release => (),
            },
            Event::Resize(width, height) => {
                resize_term(height, width, pty);
            }
            _ => (),
        }
    }

    Ok(false)
}

fn handle_key_press<W: Write>(key: KeyEvent, stdin: &mut W) -> io::Result<bool> {
    match key.code {
        KeyCode::F(4) => {
            return Ok(true);
        }
        KeyCode::Char(c) => {
            let mut b = [0u8; 4];
            let s = c.encode_utf8(&mut b);
            stdin.write_all(s.as_bytes())?;
        }
        KeyCode::Esc => stdin.write_all(&[0x1B])?,
        KeyCode::Enter => stdin.write_all(&[0x0A])?,
        KeyCode::Backspace => stdin.write_all(&[0x08])?,
        KeyCode::Delete => stdin.write_all(&[0x7F])?,
        KeyCode::Up => stdin.write_all(&[0x1B, 0x5B, 0x41])?,
        KeyCode::Down => stdin.write_all(&[0x1B, 0x5B, 0x42])?,
        KeyCode::Right => stdin.write_all(&[0x1B, 0x5B, 0x43])?,
        KeyCode::Left => stdin.write_all(&[0x1B, 0x5B, 0x44])?,
        _key => (),
    }

    stdin.flush()?;

    Ok(false)
}

fn resize_term(rows: u16, cols: u16, pty: &VT100) {
    pty.write().set_size(rows, cols);
}

fn ui(frame: &mut Frame, screen: &vt100::Screen) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(&[
            Constraint::Percentage(0),
            Constraint::Percentage(100),
            Constraint::Min(1),
        ])
        .split(frame.size());

    let block = Block::default()
        .borders(Borders::ALL)
        .title("[Running: VM]")
        .style(Style::default().add_modifier(Modifier::BOLD));

    let pty = PseudoTerminal::new(screen).block(block);
    frame.render_widget(pty, chunks[1]);

    let block = Block::default().borders(Borders::ALL);
    frame.render_widget(block, frame.size());

    let explanation = Paragraph::new("Press F4 to quit")
        .style(Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED))
        .alignment(Alignment::Center);

    frame.render_widget(explanation, chunks[2]);
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
    let parser = Arc::new(RwLock::new(vt100::Parser::new(24, 80, 0)));

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
