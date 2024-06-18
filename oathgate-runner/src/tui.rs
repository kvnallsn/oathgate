//! Terminal User Interface

mod events;

use std::os::unix::thread::JoinHandleExt;

use crossterm::{
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use nix::sys::signal::Signal;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use tui_term::widget::PseudoTerminal;

use crate::{hypervisor::{ArcTerminalMap, Hypervisor}, HypervisorError};

use self::events::EventHandler;

/// Runs the terminal user interface (TUI)
///
/// This will start the hypervisor in a separate (background) thread and runs the TUI on the main
/// thread.
///
/// ### Arguments
/// * `hypervisor` - Hypervisor (and vm) this tui will control
pub fn run(mut hypervisor: Hypervisor) -> Result<(), HypervisorError> {
    let terminals = hypervisor.terminals();

    // spawn a thread to handle the vm
    let handle = std::thread::Builder::new()
        .name(String::from("qemu-vm"))
        .spawn(move || {
            if let Err(error) = hypervisor.run() {
                tracing::error!(?error, "unable to run qemu vm");
            }
        })?;

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

    nix::sys::pthread::pthread_kill(handle.as_pthread_t(), Signal::SIGTERM)?;

    match handle.join() {
        Ok(_) => tracing::info!("vm stopped"),
        Err(error) => tracing::error!(?error, "unable to stop vm"),
    }

    Ok(())
}

/// Renders the UI part of the TUI
///
/// ### Arguments
/// * `frame` - Frame / area to render tui
/// * `terminals` - Reference to the terminal map
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
