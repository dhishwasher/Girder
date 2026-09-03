"use strict";

// Shared binary location and platform mapping for the npm wrapper.
//
// The wrapper exists because MCP clients configure servers as commands, and
// the convention across agent hosts is `npx -y <package>`. Requiring a Rust
// toolchain instead would exclude most of the people who would use this.

const fs = require("fs");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");

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
    "  git clone https://github.com/dhishwasher/Bit-code\n" +
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

// Set to "1" to skip the PATH search entirely and require the binary this
// package downloaded. Release validation otherwise silently exercises
// whatever `bitcode` a developer happens to have on PATH instead of the
// package under test — see "Validating a release" in npm/README.md.
const FORCE_VENDORED_ENV = "BITCODE_FORCE_VENDORED";

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
 *
 * `BITCODE_FORCE_VENDORED=1` overrides both: it exists so that testing the
 * published package on a machine that already has a `bitcode` on PATH (any
 * developer's machine, generally) actually tests the vendored download
 * instead of silently re-testing whatever is on PATH. It throws rather than
 * falling back, because a silent fallback here would defeat the point.
 */
function resolveBinary() {
  if (process.env[FORCE_VENDORED_ENV] === "1") {
    const vendored = vendoredPath();
    if (!fs.existsSync(vendored)) {
      throw new Error(
        `${FORCE_VENDORED_ENV}=1 but no vendored binary at ${vendored} ` +
          "(the postinstall download may not have run yet)"
      );
    }
    return vendored;
  }
  if (process.env[REENTRY_ENV] !== "1") {
    const onPath = fromPath();
    if (onPath) {
      return onPath;
    }
  }
  const vendored = vendoredPath();
  return fs.existsSync(vendored) ? vendored : null;
}

/** `<binary> --version` output, or a placeholder if it could not be run. */
function versionOf(binaryPath) {
  try {
    return execFileSync(binaryPath, ["--version"], {
      encoding: "utf8",
      timeout: 5000,
    }).trim();
  } catch (error) {
    return `<could not run --version: ${error.message}>`;
  }
}

/**
 * A stderr line naming which binary won and why, plus — when a PATH binary
 * was chosen and a vendored one also exists — a second line naming both
 * paths and both `--version` outputs, so a version skew like the one in
 * CLAUDE.md (a stale `~/.cargo/bin/bitcode` silently shadowing a freshly
 * downloaded release) is visible without running `which bitcode` by hand.
 * Returns lines rather than writing them, so callers keep control of which
 * stream they land on (always stderr — stdout is the JSON-RPC stream).
 */
function resolutionNotice(prefix, binary) {
  const vendored = vendoredPath();
  const lines = [];
  if (binary === vendored) {
    lines.push(`${prefix}: using ${binary} (downloaded by this package)`);
    return lines;
  }
  lines.push(`${prefix}: using ${binary} (found on PATH, which takes precedence)`);
  if (fs.existsSync(vendored)) {
    lines.push(
      `${prefix}: a downloaded copy also exists at ${vendored}. ` +
        `PATH: ${binary} --version -> "${versionOf(binary)}" | ` +
        `vendored: ${vendored} --version -> "${versionOf(vendored)}"`
    );
  }
  return lines;
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
  FORCE_VENDORED_ENV,
  REENTRY_ENV,
  TARGETS,
  binaryName,
  childEnv,
  resolutionNotice,
  resolveBinary,
  target,
  unsupportedMessage,
  vendoredPath,
  versionOf,
};
