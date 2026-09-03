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

// Set on the child so a shim can tell it was launched by another shim. See
// `resolveBinary`.
const REENTRY_ENV = "BITCODE_NPM_SHIM";

/**
 * The binary to execute, or null if none is available.
 *
 * A binary already on PATH wins over the vendored download. That ordering is
 * deliberate: someone who built from source, or installed a newer release
 * with install.sh, should not silently get an older vendored copy. It also
 * makes the wrapper work when the postinstall download was blocked by a
 * network policy but the tool is installed anyway.
 *
 * The exception is a shim launched by another shim, which must not consult
 * PATH at all: whatever it found there is what launched us, so looking again
 * returns the same answer forever. Going straight to the vendored binary
 * terminates the chain with the right program rather than an error.
 */
function resolveBinary() {
  if (process.env[REENTRY_ENV] !== "1") {
    const onPath = fromPath();
    if (onPath) {
      return onPath;
    }
  }
  const vendored = vendoredPath();
  return fs.existsSync(vendored) ? vendored : null;
}

/**
 * Environment for the spawned child, marking that a shim is now in the chain.
 */
function childEnv() {
  return { ...process.env, [REENTRY_ENV]: "1" };
}

function realpathOrResolve(candidate) {
  try {
    return fs.realpathSync(candidate);
  } catch {
    return path.resolve(candidate);
  }
}

function fromPath() {
  const entries = (process.env.PATH || "").split(path.delimiter);
  const name = binaryName();
  const ownBin = realpathOrResolve(path.join(__dirname, "bin"));
  for (const entry of entries) {
    if (!entry) {
      continue;
    }
    const candidate = path.join(entry, name);
    try {
      fs.accessSync(candidate, fs.constants.X_OK);
      if (!fs.statSync(candidate).isFile()) {
        continue;
      }
    } catch {
      // Not here; keep looking.
      continue;
    }
    // Judge the candidate by where it actually points, not by the PATH entry
    // it was found under. `npm install -g bitcode-mcp` installs a `bitcode`
    // symlink into a global bin directory that resolves back into this
    // package, so spawning it is spawning ourselves — an unbounded chain of
    // node processes. Comparing the PATH entry alone missed that, because the
    // global bin directory is not this package's bin directory.
    if (path.dirname(realpathOrResolve(candidate)) === ownBin) {
      continue;
    }
    return candidate;
  }
  return null;
}

module.exports = {
  REENTRY_ENV,
  TARGETS,
  binaryName,
  childEnv,
  resolveBinary,
  target,
  unsupportedMessage,
  vendoredPath,
};
