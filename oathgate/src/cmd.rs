//! Command line interface and options

mod bridge;
mod shard;

use dialoguer::Confirm;
use nix::{sys::signal::Signal, unistd::Pid};

use crate::State;

pub use self::{bridge::BridgeCommand, shard::ShardCommand};

/// Helper function to prompt for confirmation and, if approved, stop a process
///
/// ### Arguments
/// * `state` - Application state
/// * `pid` - Process id of process to stop
/// * `prompt` - Prompt to display in confirmation prompt
pub(crate) fn stop_process(state: &State, pid: i32, prompt: &str) -> anyhow::Result<bool> {
    let confirmation =
        state.skip_confirm() || Confirm::new().with_prompt(prompt).interact()?;

    if confirmation {
        // send a sigterm to the process
        nix::sys::signal::kill(Pid::from_raw(pid), Signal::SIGTERM)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub trait AsTable {
    fn header() -> &'static [&'static str];
    fn update_col_width(&self, widths: &mut [usize]);
    fn as_table_row(&self, widths: &[usize]);
}

pub fn draw_table<T: AsTable>(rows: &[T]) {
    let term = console::Term::stdout();

    let mut col_widths = T::header().iter().map(|h| h.len()).collect::<Vec<_>>();

    for row in rows {
        row.update_col_width(&mut col_widths);
    }

    let draw_separator = || {
        for &width in &col_widths {
            print!("+{}", "-".repeat(width + 2));
        }
        println!("+");
    };

    draw_separator();
    print!("|");
    for (i, header) in T::header().iter().enumerate() {
        print!(" {:width$} |", header, width = col_widths[i]);
    }
    println!();
    draw_separator();

    for row in rows {
        print!("|");
        row.as_table_row(&col_widths);
        println!();
    }

    draw_separator();

    term.flush().ok();
}
