//! Event Handler

use std::{
    io::{self, Write},
    time::Duration,
};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::VT100;

/// An `EventHandler` contains state for keyboard events
#[derive(Default, Debug)]
pub struct EventHandler {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl EventHandler {
    pub fn handle<W: Write>(&mut self, pty: &VT100, stdin: &mut W) -> io::Result<bool> {
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => match key.kind {
                    KeyEventKind::Press => {
                        return match key.modifiers {
                            KeyModifiers::CONTROL => self.handle_ctrl_key_press(key, stdin),
                            KeyModifiers::NONE => self.handle_key_press(key, stdin),
                            _ => Ok(false),
                        };
                    }
                    KeyEventKind::Release => (),
                    KeyEventKind::Repeat => (),
                },
                Event::Resize(width, height) => {
                    self.resize_term(height, width, pty);
                }
                _ => (),
            }
        }

        Ok(false)
    }

    fn handle_key_press<W: Write>(&mut self, key: KeyEvent, stdin: &mut W) -> io::Result<bool> {
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
            KeyCode::Tab => stdin.write_all(&[0x09])?,
            KeyCode::Up => stdin.write_all(&[0x1B, 0x5B, 0x41])?,
            KeyCode::Down => stdin.write_all(&[0x1B, 0x5B, 0x42])?,
            KeyCode::Right => stdin.write_all(&[0x1B, 0x5B, 0x43])?,
            KeyCode::Left => stdin.write_all(&[0x1B, 0x5B, 0x44])?,
            _key => (),
        }

        stdin.flush()?;

        Ok(false)
    }

    fn handle_ctrl_key_press<W: Write>(
        &mut self,
        key: KeyEvent,
        stdin: &mut W,
    ) -> io::Result<bool> {
        match key.code {
            KeyCode::Char('d') => stdin.write_all(&[0x04])?,
            KeyCode::Char('r') => stdin.write_all(&[0x12])?,
            KeyCode::Char('u') => stdin.write_all(&[0x15])?,
            KeyCode::Char('z') => stdin.write_all(&[0x1A])?,
            _ => (),
        }
        Ok(false)
    }

    fn resize_term(&self, rows: u16, cols: u16, pty: &VT100) {
        pty.write().set_size(rows, cols);
    }
}
