//! Shared `--out <path>` sink for `review`, `analyze`, `test-impact`, and
//! `plan run`: the full report goes to a buffer that gets written to the
//! file, while a short summary line still always goes to real stdout. When
//! no `--out` is given, `Sink::Stdout` makes every call behave exactly like
//! the `println!` it replaced — default output is unchanged.

/// Where a command's line-oriented output goes. `Buffer` accumulates lines
/// (each call adds exactly one trailing `\n`, matching `println!`) so the
/// whole report can be written to a file in one `std::fs::write`.
pub(crate) enum Sink {
    Stdout,
    Buffer(String),
}

impl Sink {
    pub(crate) fn emit(&mut self, args: std::fmt::Arguments) {
        match self {
            Sink::Stdout => println!("{args}"),
            Sink::Buffer(buf) => {
                use std::fmt::Write as _;
                let _ = writeln!(buf, "{args}");
            }
        }
    }

    /// Write the accumulated buffer to `path` if this is a `Buffer` sink; a
    /// no-op for `Stdout`. Returns whether anything was written, so a
    /// caller can decide whether to mention the path in its summary line.
    pub(crate) fn finish(self, path: Option<&std::path::Path>) -> std::io::Result<()> {
        match (self, path) {
            (Sink::Buffer(buf), Some(path)) => std::fs::write(path, buf),
            _ => Ok(()),
        }
    }
}

/// `out!(sink, "...", args...)` — drop-in replacement for `println!` that
/// respects `--out` redirection.
macro_rules! out {
    ($sink:expr, $($arg:tt)*) => {
        $sink.emit(format_args!($($arg)*))
    };
}
pub(crate) use out;
