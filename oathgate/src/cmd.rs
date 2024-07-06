//! Command line interface and options

mod bridge;
mod kernel;
mod shard;
mod template;

use std::{borrow::Cow, fmt::Display, time::Duration, io::Read};

use anyhow::anyhow;
use clap::{Args, ValueEnum};
use console::style;
use dialoguer::Confirm;
use indicatif::{ProgressBar, ProgressStyle};
use uuid::Uuid;

use crate::{database::log::LogEntry, logger::LogLevel, State};

pub use self::{bridge::BridgeCommand, shard::ShardCommand, template::TemplateCommand, kernel::KernelCommand};

#[derive(Args, Debug)]
pub struct LogSettings {
    /// Format to save logs
    #[clap(long)]
    pub log_format: LogFormat,

    /// Verbosity of spawned/detached process
    #[clap(long)]
    pub log_level: LogLevel,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum LogFormat {
    /// Pretty print logs (for human consumption)
    Pretty,

    /// Output as json (for machine consumption)
    Json,
}

impl Display for LogFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            Self::Pretty => "pretty",
            Self::Json => "json",
        };

        write!(f, "{str}")
    }
}

/// Instructions on how to render a struct as a row in an ascii table
pub trait AsTable {
    fn header() -> &'static [&'static str];
    fn update_col_width(&self, widths: &mut [usize]);
    fn as_table_row(&self, widths: &[usize]);

    fn print_field<D: Display>(&self, field: D, width: usize) {
        print!(" {:width$} \u{2502}", field, width = width);
    }
}

/// Draws an ascii-render table to stdout
///
/// ### Arguments
/// * `rows` - Slice of rows to render
pub fn draw_table<T: AsTable>(rows: &[T]) {
    let term = console::Term::stdout();

    let mut col_widths = T::header().iter().map(|h| h.len()).collect::<Vec<_>>();

    for row in rows {
        row.update_col_width(&mut col_widths);
    }

    let pipe = "\u{2502}";

    let draw_separator = |line: &str| {
        let (l, m, e) = match line {
            "first" => ("\u{250c}", "\u{252c}", "\u{2510}"),
            "last" => ("\u{2514}", "\u{2534}", "\u{2518}"),
            _ => ("\u{251c}", "\u{253c}", "\u{2524}"),
        };

        let mut first = true;

        for &width in &col_widths {
            let sep = match first {
                true => {
                    first = false;
                    l
                }
                false => m,
            };

            print!("{sep}{}", "\u{2500}".repeat(width + 2));
        }
        println!("{e}");
    };

    draw_separator("first");
    print!("{pipe}");
    for (i, header) in T::header().iter().enumerate() {
        print!(
            " {:width$} {pipe}",
            style(header).dim(),
            width = col_widths[i]
        );
    }
    println!();
    draw_separator("middle");

    for row in rows {
        print!("{pipe}");
        row.as_table_row(&col_widths);
        println!();
    }

    draw_separator("last");

    term.flush().ok();
}

/// Prints logs to the terminal for the corresponding id
///
/// ### Arguments
/// * `state` - Application state
/// * `id` - Id of device/shard/etc. to print logs for
/// * `format` - Format to print logs
pub(crate) fn print_logs(state: &State, id: Uuid, format: LogFormat) -> anyhow::Result<()> {
    let logs = LogEntry::get(state.db(), id)?;

    match format {
        LogFormat::Pretty => {
            for log in logs {
                log.display();
            }
        }
        LogFormat::Json => {
            let json = serde_json::to_string(&logs)?;
            println!("{json}");
        }
    }

    Ok(())
}

/// Prompts the user to confirm an action if the auto-confirm flag was not set.  Returns true if
/// the user accepted/agreed to the prompt, false in all other cases.
///
/// ### Arguments
/// * `state` - Application state
/// * `prompt` - Prompt to display
pub(crate) fn confirm<S: Into<String>>(state: &State, prompt: S) -> anyhow::Result<()> {
    match state.skip_confirm() || Confirm::new().with_prompt(prompt).interact()? {
        true => Ok(()),
        false => Err(anyhow!("user cancelled operation")),
    }
}

/// Creates a new indefinite spinner to show unbound progress
///
/// ### Arguments
/// * `msg` - Message to display next to spinner
pub(crate) fn spinner<S: Into<Cow<'static, str>>>(msg: S) -> ProgressBar {
    let style = ProgressStyle::with_template("{spinner:.blue} {msg}").unwrap();
    let style = style.tick_strings(&["\u{25C9}", "\u{25CB}", "\u{25C9}"]);

    let bar = ProgressBar::new_spinner();
    bar.enable_steady_tick(Duration::from_millis(500));
    bar.set_message(msg);
    bar.set_style(style);
    bar
}

/// Prints a warning message to the terminal
pub(crate) fn warning<S: Into<Cow<'static, str>>>(msg: S) {
    let style = console::Style::new().yellow();
    let msg = style.apply_to(msg.into());
    println!("{msg}");
}

/// Hashes the content of a file
pub(crate) fn hash_file<R: Read>(rdr: &mut R) -> anyhow::Result<String> {
    use sha3::{Shake128, digest::{Update, ExtendableOutput, XofReader}};

    const BUF_SZ: usize = 4096;

    let mut buf = [0u8; BUF_SZ];
    let mut hasher = Shake128::default();

    loop {
        let sz = rdr.read(&mut buf)?;
        hasher.update(&buf[..sz]);

        if sz < BUF_SZ {
            break;
        }
    }

    let mut output = [0u8; 6];
    let mut xof = hasher.finalize_xof();
    XofReader::read(&mut xof, &mut output);

    Ok(base32::encode(base32::Alphabet::Crockford, &output))
}
