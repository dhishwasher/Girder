#!/usr/bin/env node
"use strict";

// `npx girder-mcp [dir]` starts MCP; `npx girder-mcp setup` configures agents.
//
// This is the entry point an MCP client config points at. It execs the real
// binary with `mcp` prepended, so the client's stdin/stdout are handed
// straight to the server: this process must not read, write, or buffer the
// protocol stream itself.

const { spawn } = require("child_process");
const { childEnv, resolutionNotice, resolveBinary } = require("../resolve");

let binary;
try {
  binary = resolveBinary();
} catch (error) {
  process.stderr.write(`girder-mcp: ${error.message}\n`);
  process.exit(1);
}
if (!binary) {
  process.stderr.write(
    "girder-mcp: no girder binary found.\n" +
      "The postinstall download may have been blocked. Install one with:\n" +
      "  curl -fsSL https://raw.githubusercontent.com/dhishwasher/Girder/main/install.sh | sh\n" +
      "or build from source:\n" +
      "  cargo install --path crates/aether-app\n"
  );
  process.exit(1);
}

// Say which binary won, because `resolveBinary` prefers one already on PATH
// over the version this package downloaded. That ordering is deliberate, but
// it means `npx girder-mcp@X` can run a different build entirely, and
// CLAUDE.md records a stale `~/.cargo/bin/girder` silently invalidating
// verification here before. These stderr lines make the skew visible instead
// of leaving `which girder` as the only way to notice.
for (const line of resolutionNotice("girder-mcp", binary)) {
  process.stderr.write(`${line}\n`);
}

// Default to the working directory the client launched us in, which is the
// project the agent is working on.
const args = process.argv.slice(2);
const forwarded = args.length > 0 ? args : ["."];
const command = forwarded[0] === "setup" ? forwarded : ["mcp", ...forwarded];

const child = spawn(binary, command, {
  stdio: "inherit",
  env: childEnv(),
});

child.on("error", (error) => {
  process.stderr.write(`girder-mcp: could not start ${binary}: ${error.message}\n`);
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
