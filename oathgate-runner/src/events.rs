//! Event Handler

use std::{
    io::{self, Write},
    time::Duration,
};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::VT100;

macro_rules! datamsg {
    ($c0:expr) => { [0x01, 0x01, 0x00, $c0] };
    ($c0:expr, $c1:expr) => { [0x01, 0x02, 0x00, $c0, $c1] };
    ($c0:expr, $c1:expr, $c2:expr) => { [0x01, 0x03, 0x00, $c0, $c1, $c2] };
    ($c0:expr, $c1:expr, $c2:expr, $c3:expr) => { [0x01, 0x04, 0x00, $c0, $c1, $c2, $c3] };
}

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
                    self.resize_term(height, width, pty, stdin);
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
                let mut b = [0u8; 7];
                b[0] = 0x01;
                let len = {
                    let s = c.encode_utf8(&mut b[3..]);
                    s.len()
                };
                b[1..3].copy_from_slice(&len.to_le_bytes()[..2]);
                stdin.write_all(&b[..(len + 3)])?;
            }
            KeyCode::Esc => stdin.write_all(&datamsg!(0x1B))?,
            KeyCode::Enter => stdin.write_all(&datamsg!(0x0A))?,
            KeyCode::Backspace => stdin.write_all(&datamsg!(0x08))?,
            KeyCode::Delete => stdin.write_all(&datamsg!(0x7F))?,
            KeyCode::Tab => stdin.write_all(&datamsg!(0x09))?,
            KeyCode::Up => stdin.write_all(&datamsg!(0x1B, 0x5B, 0x41))?,
            KeyCode::Down => stdin.write_all(&datamsg!(0x1B, 0x5B, 0x42))?,
            KeyCode::Right => stdin.write_all(&datamsg!(0x1B, 0x5B, 0x43))?,
            KeyCode::Left => stdin.write_all(&datamsg!(0x1B, 0x5B, 0x44))?,
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
            KeyCode::Char('d') => stdin.write_all(&datamsg!(0x04))?,
            KeyCode::Char('r') => stdin.write_all(&datamsg!(0x12))?,
            KeyCode::Char('u') => stdin.write_all(&datamsg!(0x15))?,
            KeyCode::Char('z') => stdin.write_all(&datamsg!(0x1A))?,
            _ => (),
        }
        Ok(false)
    }

    fn resize_term<W: Write>(&self, rows: u16, cols: u16, pty: &VT100, stdin: &mut W) {
        let mut buf = [0x02, 0x00, 0x00, 0x00, 0x00];
        buf[1..3].copy_from_slice(&rows.to_le_bytes());
        buf[3..5].copy_from_slice(&cols.to_le_bytes());
        pty.write().set_size(rows, cols);
        stdin.write_all(&buf).ok();
    }
}
