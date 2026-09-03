"use strict";

// End-to-end tests for bin/bitcode-mcp.js's observability line.
//
// The one thing that must never happen is wrapper output landing on stdout:
// stdout is the JSON-RPC stream an MCP client parses frame by frame, so any
// wrapper text mixed into it breaks the client. These tests spawn the real
// shim script against fixture binaries and check both streams.

const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawn } = require("node:child_process");

const SOURCE = path.join(__dirname, "..");

function makePackage() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "bitcode-shim-test-"));
  const pkg = path.join(root, "package");
  fs.mkdirSync(path.join(pkg, "bin"), { recursive: true });
  fs.copyFileSync(path.join(SOURCE, "resolve.js"), path.join(pkg, "resolve.js"));
  fs.copyFileSync(
    path.join(SOURCE, "bin", "bitcode-mcp.js"),
    path.join(pkg, "bin", "bitcode-mcp.js")
  );
  return { root, pkg };
}

function scratchDir(root, name) {
  const dir = path.join(root, name);
  fs.mkdirSync(dir, { recursive: true });
  return dir;
}

/**
 * A stand-in for the real `bitcode` binary: on `--version` it prints a
 * version line, and on `mcp <dir>` it prints one fixed line that stands in
 * for the JSON-RPC stream, so the test can check that line survives the
 * wrapper byte-for-byte.
 */
function writeFakeBitcode(dir, name, version) {
  const file = path.join(dir, "bitcode");
  const marker = `${name}-mcp-output`;
  fs.writeFileSync(
    file,
    "#!/bin/sh\n" +
      `if [ "$1" = "--version" ]; then printf 'bitcode %s\\n' "${version}"; exit 0; fi\n` +
      `printf '%s' '${marker}'\n`
  );
  fs.chmodSync(file, 0o755);
  return { file, marker };
}

function run(pkg, pathDir) {
  const child = spawn(process.execPath, [path.join(pkg, "bin", "bitcode-mcp.js"), "."], {
    cwd: pkg,
    env: { ...process.env, PATH: pathDir },
  });
  let stdout = Buffer.alloc(0);
  let stderr = "";
  child.stdout.on("data", (chunk) => {
    stdout = Buffer.concat([stdout, chunk]);
  });
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  return new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", () => resolve({ stdout, stderr }));
  });
}

test(
  "when a PATH binary wins over an existing vendored copy, stdout carries only the child's bytes and stderr names both",
  async () => {
    const { root, pkg } = makePackage();
    try {
      const vendored = writeFakeBitcode(path.join(pkg, "bin"), "vendored", "0.1.1");
      const onPathDir = scratchDir(root, "cargo-bin");
      const onPath = writeFakeBitcode(onPathDir, "on-path", "0.1.0");

      const { stdout, stderr } = await run(pkg, onPathDir);

      assert.strictEqual(
        stdout.toString("utf8"),
        onPath.marker,
        "stdout must be byte-identical to what the resolved binary wrote"
      );
      assert.doesNotMatch(stdout.toString("utf8"), /bitcode-mcp:/, "wrapper text leaked onto stdout");

      assert.match(stderr, /using/);
      assert.ok(stderr.includes(onPath.file), "stderr is missing the PATH binary's path");
      assert.ok(stderr.includes(vendored.file), "stderr is missing the vendored binary's path");
      assert.ok(stderr.includes("bitcode 0.1.0"), "stderr is missing the PATH binary's --version output");
      assert.ok(stderr.includes("bitcode 0.1.1"), "stderr is missing the vendored binary's --version output");
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  }
);

test("when only the vendored binary exists, stderr says nothing about a second binary", async () => {
  const { root, pkg } = makePackage();
  try {
    writeFakeBitcode(path.join(pkg, "bin"), "vendored", "0.1.1");
    const emptyPathDir = scratchDir(root, "empty-bin");

    const { stdout, stderr } = await run(pkg, emptyPathDir);

    assert.strictEqual(stdout.toString("utf8"), "vendored-mcp-output");
    assert.match(stderr, /downloaded by this package/);
    assert.doesNotMatch(stderr, /found on PATH/, "nothing was found on PATH to report");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
