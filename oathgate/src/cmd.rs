//! Command line interface and options

mod bridge;
mod shard;

use std::fmt::Display;

pub use self::{bridge::BridgeCommand, shard::ShardCommand};

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
        print!(" {:width$} {pipe}", header, width = col_widths[i]);
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
