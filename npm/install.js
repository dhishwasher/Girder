"use strict";

// postinstall: fetch the release binary for this platform.
//
// Never fails the install. An MCP server that cannot download is recoverable
// (the shim falls back to a `girder` already on PATH, and prints how to get
// one), but a failed postinstall aborts `npx` entirely and leaves the user
// with an error that names npm rather than the actual problem.

const fs = require("fs");
const os = require("os");
const path = require("path");
const { createHash } = require("crypto");
const { execFileSync } = require("child_process");

const { binaryName, target, unsupportedMessage, vendoredPath } = require("./resolve");

const REPO = process.env.GIRDER_REPO || "dhishwasher/Girder";
const VERSION = process.env.GIRDER_VERSION || `v${require("./package.json").version}`;
const BASE_URL =
  process.env.GIRDER_BASE_URL || `https://github.com/${REPO}/releases/download`;

function note(message) {
  // stderr: stdout of the *server* is a protocol stream, and keeping all
  // wrapper output on stderr means no install message can ever be mistaken
  // for a JSON-RPC frame.
  process.stderr.write(`girder-mcp: ${message}\n`);
}

async function download(url, destination) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText} for ${url}`);
  }
  const buffer = Buffer.from(await response.arrayBuffer());
  fs.writeFileSync(destination, buffer);
  return buffer;
}

/**
 * Throw unless `bytes` matches the checksum published beside the asset.
 *
 * Every failure mode throws, which the caller turns into a discarded download
 * rather than a failed install.
 */
async function verify(url, bytes) {
  const response = await fetch(`${url}.sha256`, { redirect: "follow" });
  if (!response.ok) {
    throw new Error(
      `no checksum at ${url}.sha256 (${response.status} ${response.statusText})`
    );
  }
  const expected = (await response.text()).trim().split(/\s+/)[0] || "";
  // A mirror can answer 200 with an error page, and calling that a mismatch
  // would point the reader at their download instead of at their mirror.
  if (!/^[0-9a-f]{64}$/i.test(expected)) {
    throw new Error(`malformed checksum file at ${url}.sha256`);
  }
  const actual = createHash("sha256").update(bytes).digest("hex");
  if (expected.toLowerCase() !== actual) {
    throw new Error(`checksum mismatch (expected ${expected}, got ${actual})`);
  }
}

async function main() {
  if (process.env.GIRDER_SKIP_DOWNLOAD === "1") {
    note("GIRDER_SKIP_DOWNLOAD=1; not downloading");
    return;
  }

  const triple = target();
  if (!triple) {
    note(unsupportedMessage());
    return;
  }

  const isWindows = os.platform() === "win32";
  const asset = isWindows ? `girder-${triple}.zip` : `girder-${triple}.tar.gz`;
  const url = `${BASE_URL}/${VERSION}/${asset}`;

  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "girder-mcp-"));
  try {
    note(`downloading ${asset} (${VERSION})`);
    const archive = path.join(scratch, asset);
    const bytes = await download(url, archive);

    // The release publishes a .sha256 beside every asset, so anything other
    // than a match means these bytes are not the ones that were released —
    // whether the checksum 404s, arrives malformed, or disagrees. All of
    // those discard the download and fall back to PATH, because the one
    // thing this must not do is install unverified bytes and then hand an
    // agent the result. Verification used to be skipped whenever the
    // checksum could not be fetched, which made a single blocked request
    // enough to turn that guarantee off.
    if (process.env.GIRDER_SKIP_CHECKSUM === "1") {
      note(`GIRDER_SKIP_CHECKSUM=1, so ${asset} was not verified`);
    } else {
      await verify(url, bytes);
    }

    // bsdtar handles both .tar.gz and .zip, and ships with macOS and
    // Windows 10+; GNU tar covers the Linux .tar.gz case. That avoids
    // taking a dependency purely to unpack one file.
    execFileSync("tar", ["-xf", archive], { cwd: scratch, stdio: "ignore" });

    const unpacked = path.join(scratch, binaryName());
    if (!fs.existsSync(unpacked)) {
      throw new Error(`archive did not contain ${binaryName()}`);
    }

    const destination = vendoredPath();
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    // Write beside the target and rename, so an interrupted copy cannot
    // leave a truncated executable in place.
    const staging = `${destination}.incoming`;
    fs.copyFileSync(unpacked, staging);
    fs.chmodSync(staging, 0o755);
    fs.renameSync(staging, destination);

    note(`installed ${destination}`);
  } catch (error) {
    note(`could not install the prebuilt binary: ${error.message}`);
    note(
      "Falling back to a `girder` on PATH. To install one:\n" +
        "  curl -fsSL https://raw.githubusercontent.com/" +
        REPO +
        "/main/install.sh | sh"
    );
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true });
  }
}

main().catch((error) => {
  note(`unexpected error: ${error.message}`);
  // Still exit 0: see the file header.
});
