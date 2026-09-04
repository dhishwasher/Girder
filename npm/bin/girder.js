#!/usr/bin/env node
"use strict";

// `npx -p girder-mcp girder <args...>` — the full Girder CLI.
//
// The MCP server is the reason most people install this package, but the
// same binary is the CLI (`analyze`, `search`, `review`, `test-impact`), and
// having installed it through npm should not mean you cannot reach that.

const { spawn } = require("child_process");
const { childEnv, resolveBinary } = require("../resolve");

let binary;
try {
  binary = resolveBinary();
} catch (error) {
  process.stderr.write(`girder: ${error.message}\n`);
  process.exit(1);
}
if (!binary) {
  process.stderr.write(
    "girder: no girder binary found.\n" +
      "The postinstall download may have been blocked. Install one with:\n" +
      "  curl -fsSL https://raw.githubusercontent.com/dhishwasher/Girder/main/install.sh | sh\n"
  );
  process.exit(1);
}

const child = spawn(binary, process.argv.slice(2), {
  stdio: "inherit",
  env: childEnv(),
});

child.on("error", (error) => {
  process.stderr.write(`girder: could not start ${binary}: ${error.message}\n`);
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
