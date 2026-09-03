#!/usr/bin/env node
"use strict";

// `npx -p bitcode-mcp bitcode <args...>` — the full Bit Code CLI.
//
// The MCP server is the reason most people install this package, but the
// same binary is the CLI (`analyze`, `search`, `review`, `test-impact`), and
// having installed it through npm should not mean you cannot reach that.

const { spawn } = require("child_process");
const { resolveBinary } = require("../resolve");

const binary = resolveBinary();
if (!binary) {
  process.stderr.write(
    "bitcode: no bitcode binary found.\n" +
      "The postinstall download may have been blocked. Install one with:\n" +
      "  curl -fsSL https://raw.githubusercontent.com/dhishwasher/bit-code/main/install.sh | sh\n"
  );
  process.exit(1);
}

const child = spawn(binary, process.argv.slice(2), { stdio: "inherit" });

child.on("error", (error) => {
  process.stderr.write(`bitcode: could not start ${binary}: ${error.message}\n`);
  process.exit(1);
});

for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => {
    if (!child.killed) {
      child.kill(signal);
    }
  });
}

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code === null ? 1 : code);
  }
});
