//! Real Python execution tracing via `sys.settrace`.
//!
//! Spawns a CPython subprocess with an embedded tracing harness that records
//! every execution event (call/line/return) as a JSON-per-line stream, then
//! parses that stream into a [`PyTimeline`].
//!
//! The what-if model mirrors the toy [`Timeline`]: fork any branch at any step,
//! inject a variable override via CPython's `PyFrame_LocalsToFast`, and watch
//! the consequences propagate forward — but now over real program execution.
//!
//! Requires `python3` on `$PATH`. Tested against CPython 3.8+.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

// ── public types ─────────────────────────────────────────────────────────────

/// One recorded Python execution event (line / call / return).
#[derive(Debug, Clone)]
pub struct PyStep {
    pub seq: usize,
    /// Absolute path of the file where this event occurred.
    pub file: String,
    pub line: u32,
    /// `"line"`, `"call"`, or `"return"`.
    pub event: String,
    /// Snapshot of non-private local variables at this event, as `repr()` strings.
    pub locals: BTreeMap<String, String>,
    /// Pre-formatted one-liner for the panel / CLI output.
    pub description: String,
    /// True when this step was the injection site of a what-if override.
    pub intervened: bool,
}

/// A linear Python execution trace — one complete run of the target file.
#[derive(Debug, Clone, Default)]
pub struct PyTrace {
    pub steps: Vec<PyStep>,
}

impl PyTrace {
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Return the `repr()`-string of `var` at its most recent occurrence.
    pub fn last_value(&self, var: &str) -> Option<&str> {
        self.steps
            .iter()
            .rev()
            .find_map(|s| s.locals.get(var).map(String::as_str))
    }
}

/// One branch in the Python what-if timeline.
pub struct PyBranch {
    pub id: usize,
    pub label: String,
    /// Absolute path of the traced file — needed when forking this branch.
    pub(crate) file: String,
    pub trace: PyTrace,
}

/// Time-travel timeline for real Python execution.
///
/// Branch 0 is always the unmodified run. Subsequent branches are what-if forks
/// created by [`PyTimeline::fork_what_if`].
pub struct PyTimeline {
    branches: Vec<PyBranch>,
}

impl PyTimeline {
    /// Run `path` under `python3` and record every execution event as branch 0.
    pub fn record(path: &Path) -> io::Result<Self> {
        let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let file = abs.to_string_lossy().into_owned();
        let trace = run_python(&abs, None)?;
        Ok(PyTimeline {
            branches: vec![PyBranch {
                id: 0,
                label: "main".to_string(),
                file,
                trace,
            }],
        })
    }

    pub fn branch(&self, id: usize) -> Option<&PyBranch> {
        self.branches.iter().find(|b| b.id == id)
    }

    pub fn branches(&self) -> &[PyBranch] {
        &self.branches
    }

    /// Fork `from_branch` at `at_step`: force `var = val` (a Python literal
    /// expression, e.g. `"42"`, `"3.14"`, `"'hello'"`) and re-run from the
    /// beginning with the injection applied at that step. Returns the new id.
    pub fn fork_what_if(
        &mut self,
        from_branch: usize,
        at_step: usize,
        var: &str,
        val: &str,
        label: &str,
    ) -> io::Result<usize> {
        let file = self
            .branches
            .iter()
            .find(|b| b.id == from_branch)
            .map(|b| b.file.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "branch not found"))?;

        let override_ = PyOverride {
            at_step,
            var: var.to_string(),
            val: val.to_string(),
        };
        let trace = run_python(Path::new(&file), Some(&override_))?;
        let id = self.branches.len();
        self.branches.push(PyBranch {
            id,
            label: label.to_string(),
            file,
            trace,
        });
        Ok(id)
    }

    /// Return the first step index where `a` and `b` diverge in their locals.
    pub fn first_divergence(&self, a: usize, b: usize) -> Option<usize> {
        let ta = &self.branch(a)?.trace;
        let tb = &self.branch(b)?.trace;
        ta.steps
            .iter()
            .zip(tb.steps.iter())
            .find(|(sa, sb)| sa.locals != sb.locals)
            .map(|(sa, _)| sa.seq)
    }
}

// ── internals ────────────────────────────────────────────────────────────────

struct PyOverride {
    at_step: usize,
    var: String,
    /// A valid Python literal expression (e.g. "42", "'hello'", "[1,2,3]").
    val: String,
}

/// JSON shape emitted by the embedded tracer script (one object per line).
#[derive(Deserialize)]
struct RawStep {
    seq: usize,
    file: String,
    line: u32,
    event: String,
    locals: BTreeMap<String, String>,
    #[serde(default)]
    intervened: bool,
}

fn run_python(path: &Path, override_: Option<&PyOverride>) -> io::Result<PyTrace> {
    let script = build_script(path, override_);
    let output = Command::new("python3")
        .arg("-c")
        .arg(&script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("could not spawn python3 — is Python 3 on $PATH? ({e})"),
            )
        })?;

    // If the process failed AND produced no output we treat it as a hard error.
    if !output.status.success() && output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!(
            "python3 exited with error:\n{stderr}"
        )));
    }

    let mut steps = Vec::new();
    for raw_line in BufReader::new(output.stdout.as_slice()).lines() {
        let raw_line = raw_line?;
        if raw_line.trim().is_empty() {
            continue;
        }
        // Non-JSON lines (e.g. print() output from the user's script) are silently skipped.
        if let Ok(raw) = serde_json::from_str::<RawStep>(&raw_line) {
            let description = format_description(&raw);
            steps.push(PyStep {
                seq: raw.seq,
                file: raw.file,
                line: raw.line,
                event: raw.event,
                locals: raw.locals,
                description,
                intervened: raw.intervened,
            });
        }
    }
    Ok(PyTrace { steps })
}

fn format_description(raw: &RawStep) -> String {
    let glyph = match raw.event.as_str() {
        "call" => "→",
        "return" => "←",
        _ => "·",
    };
    let locals_preview = raw
        .locals
        .iter()
        .take(5)
        .map(|(k, v)| {
            // Trim long reprs to keep lines readable.
            let v_short = if v.len() > 24 { &v[..24] } else { v };
            format!("{k}={v_short}")
        })
        .collect::<Vec<_>>()
        .join("  ");
    let more = if raw.locals.len() > 5 {
        format!(" +{}", raw.locals.len() - 5)
    } else {
        String::new()
    };
    let intervened_mark = if raw.intervened { "  ★ what-if" } else { "" };
    format!(
        "{glyph} line {:4}  {{{}{}}}{intervened_mark}",
        raw.line, locals_preview, more
    )
}

/// Build the inline Python tracer script.
///
/// The script installs a `sys.settrace` hook, exec's the target file in an
/// isolated namespace, then prints each recorded event as a JSON object on its
/// own line. If `override_` is given, a variable is forced to a new value at
/// the specified step via `ctypes.pythonapi.PyFrame_LocalsToFast`.
fn build_script(path: &Path, override_: Option<&PyOverride>) -> String {
    // Encode the path as a JSON string so backslashes and quotes survive.
    let path_repr = serde_json::to_string(&path.to_string_lossy().as_ref())
        .unwrap_or_else(|_| format!("\"{}\"", path.display()));

    let inject = match override_ {
        None => String::new(),
        Some(o) => {
            // Encode the variable name the same way.
            let var_repr =
                serde_json::to_string(&o.var).unwrap_or_else(|_| format!("\"{}\"", o.var));
            // o.val must be a valid Python expression supplied by the caller.
            format!(
                "\n    if _seq[0] == {at_step} and {var_repr} in frame.f_locals:\
                 \n        try:\
                 \n            import ctypes as _ct\
                 \n            frame.f_locals[{var_repr}] = {val}\
                 \n            _ct.pythonapi.PyFrame_LocalsToFast(_ct.py_object(frame), _ct.c_int(0))\
                 \n            _intervened = True\
                 \n        except Exception:\
                 \n            pass",
                at_step = o.at_step,
                var_repr = var_repr,
                val = o.val,
            )
        }
    };

    // Python braces need doubling inside Rust's format! macro: {{}} → {}.
    // settrace is installed AFTER reading and compiling the file so that the
    // codec / IO internals aren't captured in the trace — only exec'd code is.
    format!(
        r#"import sys as _sys, json as _json
_steps = []
_seq = [0]

def _tracer(frame, event, arg):
    if event not in ('line', 'call', 'return'):
        return _tracer
    _intervened = False{inject}
    _locs = {{}}
    for _k, _v in frame.f_locals.items():
        if _k.startswith('_'):
            continue
        try:
            _locs[_k] = repr(_v)
        except Exception:
            _locs[_k] = '<err>'
    _steps.append({{
        'seq': _seq[0],
        'file': frame.f_code.co_filename,
        'line': frame.f_lineno,
        'event': event,
        'locals': _locs,
        'intervened': _intervened,
    }})
    _seq[0] += 1
    return _tracer

_path = {path_repr}
with open(_path) as _f:
    _code = compile(_f.read(), _path, 'exec')
_sys.settrace(_tracer)
exec(_code, {{}})
_sys.settrace(None)

for _s in _steps:
    print(_json.dumps(_s))
"#,
        inject = inject,
        path_repr = path_repr,
    )
}
