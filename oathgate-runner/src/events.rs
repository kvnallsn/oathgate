//! Event Handler

use std::{io, time::Duration};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::ArcTerminalMap;

/// An `EventHandler` contains state for keyboard events
#[derive(Default, Debug)]
pub struct EventHandler {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl EventHandler {
    pub fn handle(&mut self, terminals: &ArcTerminalMap) -> io::Result<bool> {
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => match key.kind {
                    KeyEventKind::Press => {
                        return match key.modifiers {
                            KeyModifiers::CONTROL => self.handle_ctrl_key_press(key, terminals),
                            KeyModifiers::NONE => self.handle_key_press(key, terminals),
                            _ => Ok(false),
                        };
                    }
                    KeyEventKind::Release => (),
                    KeyEventKind::Repeat => (),
                },
                Event::Resize(width, height) => {
                    self.resize_term(height, width, terminals);
                }
                _ => (),
            }
        }

        Ok(false)
    }

    fn handle_key_press(&mut self, key: KeyEvent, terminals: &ArcTerminalMap) -> io::Result<bool> {
        let term = terminals.read();

        match key.code {
            KeyCode::F(4) => {
                return Ok(true);
            }
            KeyCode::Char(c) => {
                let mut b = [0u8; 4];
                let s = c.encode_utf8(&mut b);
                term.write_to_pty(s.as_bytes())?
            }
            KeyCode::Esc => term.write_to_pty(&[0x1B])?,
            KeyCode::Enter => term.write_to_pty(&[0x0A])?,
            KeyCode::Backspace => term.write_to_pty(&[0x08])?,
            KeyCode::Delete => term.write_to_pty(&[0x7F])?,
            KeyCode::Tab => term.write_to_pty(&[0x09])?,
            KeyCode::Up => term.write_to_pty(&[0x1B, 0x5B, 0x41])?,
            KeyCode::Down => term.write_to_pty(&[0x1B, 0x5B, 0x42])?,
            KeyCode::Right => term.write_to_pty(&[0x1B, 0x5B, 0x43])?,
            KeyCode::Left => term.write_to_pty(&[0x1B, 0x5B, 0x44])?,
            _key => (),
        }

        Ok(false)
    }

    fn handle_ctrl_key_press(
        &mut self,
        key: KeyEvent,
        terminals: &ArcTerminalMap
    ) -> io::Result<bool> {
        let term = terminals.read();

        match key.code {
            KeyCode::Char('d') => term.write_to_pty(&[0x04])?,
            KeyCode::Char('r') => term.write_to_pty(&[0x12])?,
            KeyCode::Char('u') => term.write_to_pty(&[0x15])?,
            KeyCode::Char('z') => term.write_to_pty(&[0x1A])?,
            _ => (),
        }
        Ok(false)
    }

    fn resize_term(&self, rows: u16, cols: u16, terminals: &ArcTerminalMap) {
        let terms = terminals.read();
        for term in terms.all() {
            if let Err(error) = term.resize_pty(rows, cols) {
                tracing::warn!(?error, "unable to resize terminal");
            }
        }
    }
}
