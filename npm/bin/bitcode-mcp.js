#!/usr/bin/env node
"use strict";

// `npx bitcode-mcp [dir]` — start the Bit Code MCP server on stdio.
//
// This is the entry point an MCP client config points at. It execs the real
// binary with `mcp` prepended, so the client's stdin/stdout are handed
// straight to the server: this process must not read, write, or buffer the
// protocol stream itself.

const { spawn } = require("child_process");
const { childEnv, resolveBinary, vendoredPath } = require("../resolve");

const binary = resolveBinary();
if (!binary) {
  process.stderr.write(
    "bitcode-mcp: no bitcode binary found.\n" +
      "The postinstall download may have been blocked. Install one with:\n" +
      "  curl -fsSL https://raw.githubusercontent.com/dhishwasher/Bit-code/main/install.sh | sh\n" +
      "or build from source:\n" +
      "  cargo install --path crates/aether-app\n"
  );
  process.exit(1);
}

// Say which binary won, because `resolveBinary` prefers one already on PATH
// over the version this package downloaded. That ordering is deliberate, but
// it means `npx bitcode-mcp@X` can run a different build entirely, and
// CLAUDE.md records a stale `~/.cargo/bin/bitcode` silently invalidating
// verification here before. One stderr line makes the skew visible instead
// of leaving `which bitcode` as the only way to notice.
process.stderr.write(
  `bitcode-mcp: using ${binary} (${binary === vendoredPath() ? "downloaded by this package" : "found on PATH, which takes precedence"})\n`
);

// Default to the working directory the client launched us in, which is the
// project the agent is working on.
const args = process.argv.slice(2);
const forwarded = args.length > 0 ? args : ["."];

const child = spawn(binary, ["mcp", ...forwarded], {
  stdio: "inherit",
  env: childEnv(),
});

child.on("error", (error) => {
  process.stderr.write(`bitcode-mcp: could not start ${binary}: ${error.message}\n`);
  process.exit(1);
});

// Forward termination so a client killing the wrapper does not orphan the
// server holding the project open.
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => {
    if (!child.killed) {
      child.kill(signal);
    }
  });
}

child.on("exit", (code, signal) => {
  // Reproduce the child's fate rather than always exiting 0, so a client's
  // restart logic sees what actually happened.
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code === null ? 1 : code);
  }
});
