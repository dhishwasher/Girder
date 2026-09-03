"use strict";

// Shared binary location and platform mapping for the npm wrapper.
//
// The wrapper exists because MCP clients configure servers as commands, and
// the convention across agent hosts is `npx -y <package>`. Requiring a Rust
// toolchain instead would exclude most of the people who would use this.

const fs = require("fs");
const os = require("os");
const path = require("path");

// Rust target triples, which is how release assets are named.
const TARGETS = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "win32-x64": "x86_64-pc-windows-msvc",
};

function target() {
  return TARGETS[`${os.platform()}-${os.arch()}`];
}

/** Human-readable reason this platform has no prebuilt binary. */
function unsupportedMessage() {
  return (
    `No prebuilt bitcode binary for ${os.platform()}-${os.arch()}.\n` +
    `Supported: ${Object.keys(TARGETS).join(", ")}.\n` +
    "Build from source instead:\n" +
    "  git clone https://github.com/dhishwasher/bit-code\n" +
    "  cargo install --path bit-code/crates/aether-app"
  );
}

function binaryName() {
  return os.platform() === "win32" ? "bitcode.exe" : "bitcode";
}

/** Where postinstall puts the downloaded binary. */
function vendoredPath() {
  return path.join(__dirname, "bin", binaryName());
}

/**
 * The binary to execute, or null if none is available.
 *
 * A binary already on PATH wins over the vendored download. That ordering is
 * deliberate: someone who built from source, or installed a newer release
 * with install.sh, should not silently get an older vendored copy. It also
 * makes the wrapper work when the postinstall download was blocked by a
 * network policy but the tool is installed anyway.
 */
function resolveBinary() {
  const onPath = fromPath();
  if (onPath) {
    return onPath;
  }
  const vendored = vendoredPath();
  return fs.existsSync(vendored) ? vendored : null;
}

function fromPath() {
  const entries = (process.env.PATH || "").split(path.delimiter);
  const name = binaryName();
  for (const entry of entries) {
    if (!entry) {
      continue;
    }
    // Skip this package's own bin directory: npx puts it on PATH, and the
    // shim there is a JS file, not the real binary.
    const candidate = path.join(entry, name);
    if (path.resolve(entry) === path.resolve(path.join(__dirname, "bin"))) {
      continue;
    }
    try {
      fs.accessSync(candidate, fs.constants.X_OK);
      const stat = fs.statSync(candidate);
      if (stat.isFile()) {
        return candidate;
      }
    } catch {
      // Not here; keep looking.
    }
  }
  return null;
}

module.exports = {
  TARGETS,
  binaryName,
  resolveBinary,
  target,
  unsupportedMessage,
  vendoredPath,
};
