"use strict";

const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const PACKAGE = path.join(__dirname, "..");
const HOOK = path.join(
  PACKAGE,
  "hooks",
  process.platform === "win32" ? "girder_context_advisory.py" : "girder_context_advisory.sh"
);
const INTERPRETER = process.platform === "win32" ? "python" : "sh";

function makeStub(root, body) {
  const stub = path.join(root, process.platform === "win32" ? "stub.cmd" : "stub-native");
  if (process.platform === "win32") {
    fs.writeFileSync(stub, `@echo off\r\n${body}\r\n`);
  } else {
    fs.writeFileSync(stub, `#!/bin/sh\n${body}\n`);
    fs.chmodSync(stub, 0o700);
  }
  return stub;
}

function runHookWithBinary(cwd, payload, binary) {
  return spawnSync(INTERPRETER, [HOOK, binary], {
    cwd,
    input: JSON.stringify(payload),
    encoding: null,
  });
}

test("launcher forwards stdin and native stdout unchanged", { skip: process.platform === "win32" }, () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "girder-hook-forward-"));
  const payload = { hook_event_name: "PreToolUse", tool_name: "Read", tool_input: { path: "src/lib.rs" } };
  const stub = makeStub(root, "cat");
  try {
    const result = runHookWithBinary(root, payload, stub);
    assert.strictEqual(result.status, 0);
    assert.deepStrictEqual(result.stdout.toString("utf8"), JSON.stringify(payload));
    assert.strictEqual(result.stderr.length, 0);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("launcher fails open when the pinned native executable is unavailable", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "girder-hook-fail-open-"));
  try {
    const result = runHookWithBinary(root, { malformed: true }, path.join(root, "missing-native"));
    assert.strictEqual(result.status, 0);
    assert.strictEqual(result.stdout.length, 0);
    assert.strictEqual(result.stderr.length, 0);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("Python launcher forwards stdin and fails open", { skip: process.platform === "win32" }, () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "girder-hook-python-"));
  const pythonHook = path.join(PACKAGE, "hooks", "girder_context_advisory.py");
  const payload = { hook_event_name: "PreToolUse", tool_name: "read_file", tool_input: { path: "src/lib.py" } };
  const stub = makeStub(root, "cat");
  try {
    const result = spawnSync("python3", [pythonHook, stub], {
      cwd: root,
      input: JSON.stringify(payload),
      encoding: null,
    });
    assert.strictEqual(result.status, 0);
    assert.deepStrictEqual(result.stdout.toString("utf8"), JSON.stringify(payload));
    assert.strictEqual(result.stderr.length, 0);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("npm package allowlist includes the advisory hook", () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(PACKAGE, "package.json"), "utf8"));
  assert.ok(manifest.files.includes("hooks/"));
  assert.ok(fs.statSync(HOOK).isFile());
  assert.ok(fs.statSync(path.join(PACKAGE, "hooks", "girder_context_advisory.py")).isFile());
});
