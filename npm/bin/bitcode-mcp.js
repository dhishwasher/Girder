#!/usr/bin/env node
"use strict";

// `npx bitcode-mcp [dir]` — start the Bit Code MCP server on stdio.
//
// This is the entry point an MCP client config points at. It execs the real
// binary with `mcp` prepended, so the client's stdin/stdout are handed
// straight to the server: this process must not read, write, or buffer the
// protocol stream itself.

const { spawn } = require("child_process");
const { resolveBinary } = require("../resolve");

const binary = resolveBinary();
if (!binary) {
  process.stderr.write(
    "bitcode-mcp: no bitcode binary found.\n" +
      "The postinstall download may have been blocked. Install one with:\n" +
      "  curl -fsSL https://raw.githubusercontent.com/dhishwasher/bit-code/main/install.sh | sh\n" +
      "or build from source:\n" +
      "  cargo install --path crates/aether-app\n"
  );
  process.exit(1);
}

// Default to the working directory the client launched us in, which is the
// project the agent is working on.
const args = process.argv.slice(2);
const forwarded = args.length > 0 ? args : ["."];

const child = spawn(binary, ["mcp", ...forwarded], { stdio: "inherit" });

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
