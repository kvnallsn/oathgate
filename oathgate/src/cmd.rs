//! Command line interface and options

mod bridge;
mod shard;

pub use self::{bridge::BridgeCommand, shard::ShardCommand};

/// Instructions on how to render a struct as a row in an ascii table
pub trait AsTable {
    fn header() -> &'static [&'static str];
    fn update_col_width(&self, widths: &mut [usize]);
    fn as_table_row(&self, widths: &[usize]);
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
