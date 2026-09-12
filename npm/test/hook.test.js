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

function runHook(cwd, payload) {
  return spawnSync(INTERPRETER, [HOOK], {
    cwd,
    input: JSON.stringify(payload),
    encoding: null,
  });
}

test("advisory hook never changes stdout and only advises for an available graph", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "girder-hook-test-"));
  const payload = {
    tool_name: "Read",
    tool_input: { file_path: path.join(root, "src", "lib.rs") },
    cwd: root,
  };
  try {
    const disabled = runHook(root, payload);
    fs.writeFileSync(path.join(root, "project.aether"), "already available\n");
    const enabled = runHook(root, payload);

    assert.strictEqual(disabled.status, 0);
    assert.strictEqual(enabled.status, 0);
    assert.deepStrictEqual(enabled.stdout, disabled.stdout);
    assert.strictEqual(enabled.stdout.length, 0, "hook leaked bytes onto stdout");
    assert.strictEqual(disabled.stderr.length, 0);
    assert.match(enabled.stderr.toString("utf8"), /^Girder: consider .+\n$/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("advisory hook fails open and silently for malformed and bounded reads", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "girder-hook-errors-"));
  try {
    fs.writeFileSync(path.join(root, "project.aether"), "already available\n");
    const malformed = spawnSync(INTERPRETER, [HOOK], {
      cwd: root,
      input: "not json",
      encoding: null,
    });
    const bounded = runHook(root, {
      tool_name: "Read",
      tool_input: { file_path: "src/lib.rs", offset: 10, limit: 20 },
      cwd: root,
    });
    for (const result of [malformed, bounded]) {
      assert.strictEqual(result.status, 0);
      assert.strictEqual(result.stdout.length, 0);
      assert.strictEqual(result.stderr.length, 0);
    }
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("Cursor workspace_roots locates a ready graph outside the hook working directory", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "girder-hook-workspace-"));
  const configDir = fs.mkdtempSync(path.join(os.tmpdir(), "girder-hook-config-"));
  try {
    fs.writeFileSync(path.join(root, "project.aether"), "already available\n");
    const result = runHook(configDir, {
      tool_name: "Read",
      tool_input: { file_path: path.join(root, "src", "lib.tsx") },
      workspace_roots: [root],
    });
    assert.strictEqual(result.status, 0);
    assert.strictEqual(result.stdout.length, 0);
    assert.match(result.stderr.toString("utf8"), /^Girder: consider .+\n$/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
    fs.rmSync(configDir, { recursive: true, force: true });
  }
});

test(
  "shell and Python hook implementations have matching observable behavior",
  { skip: process.platform === "win32" },
  () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "girder-hook-parity-"));
    const payload = {
      tool_name: "Read",
      tool_input: { file_path: path.join(root, "src", "main.go") },
      cwd: root,
    };
    try {
      fs.writeFileSync(path.join(root, "project.aether"), "already available\n");
      const shell = spawnSync(
        "sh",
        [path.join(PACKAGE, "hooks", "girder_context_advisory.sh")],
        { cwd: root, input: JSON.stringify(payload), encoding: null }
      );
      const python = spawnSync(
        "python3",
        [path.join(PACKAGE, "hooks", "girder_context_advisory.py")],
        { cwd: root, input: JSON.stringify(payload), encoding: null }
      );
      assert.strictEqual(shell.status, 0);
      assert.strictEqual(python.status, 0);
      assert.deepStrictEqual(shell.stdout, python.stdout);
      assert.deepStrictEqual(shell.stderr, python.stderr);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  }
);

test("npm package allowlist includes the advisory hook", () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(PACKAGE, "package.json"), "utf8"));
  assert.ok(manifest.files.includes("hooks/"));
  assert.ok(fs.statSync(HOOK).isFile());
  assert.ok(fs.statSync(path.join(PACKAGE, "hooks", "girder_context_advisory.py")).isFile());
});
