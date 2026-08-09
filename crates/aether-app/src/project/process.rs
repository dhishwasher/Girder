//! Fail-closed subprocess execution with hard time and output bounds.
//!
//! Every child runs in its own process group and is killed as a tree on
//! timeout, cancellation, or output overflow, so no descendant survives its
//! budget. Three capture strategies share the one engine:
//!   * [`run_captured`] — raw bytes for callers that treat output as data;
//!     exceeding the limit kills the child and reports [`BoundedStatus::
//!     OutputLimited`] rather than ever returning truncated data.
//!   * [`run_diagnostic`] — prefix/suffix-truncated human diagnostics that
//!     never kill on volume (validation output).
//!   * [`run_streamed`] — tees the child's output live to this process's
//!     stdout/stderr with a byte cap; stdin stays inherited so interactive
//!     test runners keep working.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) enum BoundedStatus {
    Completed(ExitStatus),
    TimedOut,
    OutputLimited,
    Cancelled,
}

pub(crate) struct CapturedRun {
    pub(crate) status: BoundedStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

pub(crate) struct DiagnosticRun {
    pub(crate) status: BoundedStatus,
    pub(crate) exit: Option<ExitStatus>,
    pub(crate) stdout: BoundedOutput,
    pub(crate) stderr: BoundedOutput,
}

pub(crate) struct StreamedRun {
    pub(crate) status: BoundedStatus,
}

/// Run to completion, capturing complete raw output. The child is killed and
/// `OutputLimited` reported the moment combined output exceeds `max_output_bytes`.
pub(crate) fn run_captured(
    mut command: Command,
    timeout: Duration,
    max_output_bytes: usize,
) -> std::io::Result<CapturedRun> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = spawn_grouped(command)?;
    let overflow = Arc::new(AtomicBool::new(false));
    let limit = max_output_bytes / 2;
    let stdout = raw_reader(take_stdout(&mut child)?, limit, overflow.clone());
    let stderr = raw_reader(take_stderr(&mut child)?, limit, overflow.clone());
    let status = supervise(&mut child, timeout, None, Some(&overflow))?;
    Ok(CapturedRun {
        status,
        stdout: join_reader(stdout)?,
        stderr: join_reader(stderr)?,
    })
}

/// Run to completion with prefix/suffix-truncated diagnostic capture. Output
/// volume never kills the child; time and cancellation do.
pub(crate) fn run_diagnostic(
    mut command: Command,
    timeout: Duration,
    max_output_bytes: usize,
    cancel: &Arc<AtomicBool>,
) -> std::io::Result<DiagnosticRun> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = spawn_grouped(command)?;
    let limit = max_output_bytes / 2;
    let stdout = truncating_reader(take_stdout(&mut child)?, limit);
    let stderr = truncating_reader(take_stderr(&mut child)?, limit);
    let status = supervise(&mut child, timeout, Some(cancel), None)?;
    let exit = match &status {
        BoundedStatus::Completed(exit) => Some(*exit),
        _ => child.wait().ok(),
    };
    Ok(DiagnosticRun {
        status,
        exit,
        stdout: join_reader(stdout)?,
        stderr: join_reader(stderr)?,
    })
}

/// Run to completion, teeing output live to this process's stdout/stderr.
/// The child is killed once combined output exceeds `max_output_bytes`.
pub(crate) fn run_streamed(
    mut command: Command,
    timeout: Duration,
    max_output_bytes: usize,
) -> std::io::Result<StreamedRun> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = spawn_grouped(command)?;
    let overflow = Arc::new(AtomicBool::new(false));
    let limit = max_output_bytes / 2;
    let stdout = tee_reader(take_stdout(&mut child)?, limit, overflow.clone(), false);
    let stderr = tee_reader(take_stderr(&mut child)?, limit, overflow.clone(), true);
    let status = supervise(&mut child, timeout, None, Some(&overflow))?;
    join_reader(stdout)?;
    join_reader(stderr)?;
    Ok(StreamedRun { status })
}

fn spawn_grouped(mut command: Command) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn()
}

fn take_stdout(child: &mut Child) -> std::io::Result<std::process::ChildStdout> {
    child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("child stdout was not captured"))
}

fn take_stderr(child: &mut Child) -> std::io::Result<std::process::ChildStderr> {
    child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("child stderr was not captured"))
}

/// Poll the child until exit, timeout, cancellation, or overflow. Every
/// non-completion outcome kills the entire process group first.
fn supervise(
    child: &mut Child,
    timeout: Duration,
    cancel: Option<&Arc<AtomicBool>>,
    overflow: Option<&Arc<AtomicBool>>,
) -> std::io::Result<BoundedStatus> {
    let started = Instant::now();
    loop {
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
            terminate_process_tree(child);
            let _ = child.wait();
            return Ok(BoundedStatus::Cancelled);
        }
        if overflow.is_some_and(|overflow| overflow.load(Ordering::Relaxed)) {
            terminate_process_tree(child);
            let _ = child.wait();
            return Ok(BoundedStatus::OutputLimited);
        }
        if let Some(exit) = child.try_wait()? {
            return Ok(BoundedStatus::Completed(exit));
        }
        if started.elapsed() >= timeout {
            terminate_process_tree(child);
            let _ = child.wait();
            return Ok(BoundedStatus::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub(crate) fn terminate_process_tree(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

fn raw_reader(
    mut reader: impl Read + Send + 'static,
    limit: usize,
    overflow: Arc<AtomicBool>,
) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 8192];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                return Ok(bytes);
            }
            if bytes.len() + read > limit {
                overflow.store(true, Ordering::Relaxed);
                // Keep draining so the child is not blocked on a full pipe
                // while the supervisor kills it, but stop retaining bytes.
                continue;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
    })
}

fn truncating_reader(
    mut reader: impl Read + Send + 'static,
    limit: usize,
) -> std::thread::JoinHandle<std::io::Result<BoundedOutput>> {
    std::thread::spawn(move || {
        let mut output = BoundedOutput::new(limit);
        let mut buffer = [0_u8; 8192];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                return Ok(output);
            }
            output.push(&buffer[..read]);
        }
    })
}

fn tee_reader(
    mut reader: impl Read + Send + 'static,
    limit: usize,
    overflow: Arc<AtomicBool>,
    to_stderr: bool,
) -> std::thread::JoinHandle<std::io::Result<()>> {
    std::thread::spawn(move || {
        let mut written = 0_usize;
        let mut buffer = [0_u8; 8192];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                return Ok(());
            }
            if written + read > limit {
                overflow.store(true, Ordering::Relaxed);
                continue;
            }
            written += read;
            if to_stderr {
                let mut sink = std::io::stderr();
                sink.write_all(&buffer[..read])?;
                sink.flush()?;
            } else {
                let mut sink = std::io::stdout();
                sink.write_all(&buffer[..read])?;
                sink.flush()?;
            }
        }
    })
}

fn join_reader<T>(handle: std::thread::JoinHandle<std::io::Result<T>>) -> std::io::Result<T> {
    handle
        .join()
        .map_err(|_| std::io::Error::other("subprocess output reader panicked"))?
}

/// Bounded prefix + suffix retention for human diagnostics; the middle of
/// oversized output is replaced with a truncation marker.
pub(crate) struct BoundedOutput {
    prefix: Vec<u8>,
    suffix: VecDeque<u8>,
    prefix_limit: usize,
    suffix_limit: usize,
    truncated: bool,
}

impl BoundedOutput {
    pub(crate) fn new(limit: usize) -> Self {
        let prefix_limit = limit / 2;
        Self {
            prefix: Vec::with_capacity(prefix_limit),
            suffix: VecDeque::with_capacity(limit - prefix_limit),
            prefix_limit,
            suffix_limit: limit - prefix_limit,
            truncated: false,
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        let prefix_room = self.prefix_limit.saturating_sub(self.prefix.len());
        let prefix_bytes = prefix_room.min(bytes.len());
        self.prefix.extend_from_slice(&bytes[..prefix_bytes]);
        for byte in &bytes[prefix_bytes..] {
            if self.suffix.len() == self.suffix_limit {
                self.suffix.pop_front();
                self.truncated = true;
            }
            if self.suffix_limit > 0 {
                self.suffix.push_back(*byte);
            }
        }
        self.truncated |= prefix_bytes < bytes.len() && self.suffix_limit == 0;
    }

    pub(crate) fn render(self) -> String {
        let mut bytes = self.prefix;
        if self.truncated {
            bytes.extend_from_slice(b"\n... output truncated ...\n");
        }
        bytes.extend(self.suffix);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_capture_keeps_prefix_and_suffix() {
        let mut output = BoundedOutput::new(8);
        output.push(b"abcdefghijklmnop");
        let rendered = output.render();
        assert!(rendered.starts_with("abcd"));
        assert!(rendered.contains("... output truncated ..."));
        assert!(rendered.ends_with("mnop"));
    }

    #[cfg(unix)]
    #[test]
    fn captured_run_times_out_and_kills_the_process_group() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & sleep 30"]);
        let started = Instant::now();
        let run = run_captured(command, Duration::from_millis(200), 1024 * 1024).unwrap();
        assert!(matches!(run.status, BoundedStatus::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(unix)]
    #[test]
    fn captured_run_kills_on_output_overflow_instead_of_truncating() {
        let mut command = Command::new("sh");
        command.args(["-c", "yes | head -c 1000000; sleep 30"]);
        let run = run_captured(command, Duration::from_secs(30), 64 * 1024).unwrap();
        assert!(matches!(run.status, BoundedStatus::OutputLimited));
    }

    #[cfg(unix)]
    #[test]
    fn captured_run_returns_complete_output_under_the_limit() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf hello; printf world >&2"]);
        let run = run_captured(command, Duration::from_secs(30), 64 * 1024).unwrap();
        match run.status {
            BoundedStatus::Completed(exit) => assert!(exit.success()),
            _ => panic!("expected completion"),
        }
        assert_eq!(run.stdout, b"hello");
        assert_eq!(run.stderr, b"world");
    }
}
