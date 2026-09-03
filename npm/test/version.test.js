"use strict";

// Confirms the vendored binary actually reports the version npm thinks it
// shipped: both `bitcode --version` and the MCP handshake's
// `serverInfo.version` must equal npm/package.json's version.
//
// This is the check that was missing when `npx -y bitcode-mcp` reported
// serverInfo.version 0.1.0 for a package published as 0.1.1 (the binary
// executed was a stale one from PATH, not a version mismatch in what was
// vendored) — see gap 26 in docs/core-gap-analysis.md. Resolving *which*
// binary runs is resolve.js's job and is covered by resolve.test.js; this
// suite only asks whether the binary that gets vendored agrees with the
// package that vendored it.
//
// Looks for a binary in this order:
//   1. BITCODE_TEST_BIN — an explicit path, for CI or local runs where the
//      binary under test isn't at the postinstall destination.
//   2. The vendored path a real postinstall would have populated.
// Skipped (not failed) when neither exists, the same way install.test.js
// skips when `tar` is unavailable: this suite runs via plain `node --test`,
// which implies no cargo build, and a missing binary says nothing about
// whether the two version numbers would agree.

const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const { vendoredPath } = require("../resolve");
const packageVersion = require("../package.json").version;

function findBinary() {
  if (process.env.BITCODE_TEST_BIN) {
    return process.env.BITCODE_TEST_BIN;
  }
  const vendored = vendoredPath();
  return fs.existsSync(vendored) ? vendored : null;
}

const binary = findBinary();
const skip = binary
  ? false
  : "no vendored binary found; set BITCODE_TEST_BIN or run the postinstall first";

test("the vendored binary's --version matches npm/package.json", { skip }, () => {
  const output = execFileSync(binary, ["--version"], { encoding: "utf8" }).trim();
  assert.strictEqual(
    output,
    `bitcode ${packageVersion}`,
    `binary reports a different version than npm/package.json (${packageVersion})`
  );
});

test(
  "the vendored binary's MCP serverInfo.version matches npm/package.json",
  { skip },
  () => {
    const request = JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: { protocolVersion: "2025-06-18" },
    });
    const output = execFileSync(binary, ["mcp", "."], {
      input: `${request}\n`,
      encoding: "utf8",
      cwd: path.join(__dirname, ".."),
    });
    const responseLine = output.split("\n").find((line) => line.trim().length > 0);
    assert.ok(responseLine, `no response line from the MCP handshake; got: ${output}`);
    const response = JSON.parse(responseLine);
    assert.strictEqual(
      response.result.serverInfo.version,
      packageVersion,
      `serverInfo.version disagrees with npm/package.json (${packageVersion})`
    );
  }
);
