"use strict";

// postinstall: fetch the release binary for this platform.
//
// Never fails the install. An MCP server that cannot download is recoverable
// (the shim falls back to a `bitcode` already on PATH, and prints how to get
// one), but a failed postinstall aborts `npx` entirely and leaves the user
// with an error that names npm rather than the actual problem.

const fs = require("fs");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");

const { binaryName, target, unsupportedMessage, vendoredPath } = require("./resolve");

const REPO = process.env.BITCODE_REPO || "dhishwasher/Bit-code";
const VERSION = process.env.BITCODE_VERSION || `v${require("./package.json").version}`;
const BASE_URL =
  process.env.BITCODE_BASE_URL || `https://github.com/${REPO}/releases/download`;

function note(message) {
  // stderr: stdout of the *server* is a protocol stream, and keeping all
  // wrapper output on stderr means no install message can ever be mistaken
  // for a JSON-RPC frame.
  process.stderr.write(`bitcode-mcp: ${message}\n`);
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

async function main() {
  if (process.env.BITCODE_SKIP_DOWNLOAD === "1") {
    note("BITCODE_SKIP_DOWNLOAD=1; not downloading");
    return;
  }

  const triple = target();
  if (!triple) {
    note(unsupportedMessage());
    return;
  }

  const isWindows = os.platform() === "win32";
  const asset = isWindows ? `bitcode-${triple}.zip` : `bitcode-${triple}.tar.gz`;
  const url = `${BASE_URL}/${VERSION}/${asset}`;

  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "bitcode-mcp-"));
  try {
    note(`downloading ${asset} (${VERSION})`);
    const archive = path.join(scratch, asset);
    const bytes = await download(url, archive);

    // Verify when the release publishes a checksum. A mismatch is fatal to
    // this download; a missing checksum file is not, since older releases
    // may not have one.
    try {
      const published = await fetch(`${url}.sha256`, { redirect: "follow" });
      if (published.ok) {
        const expected = (await published.text()).trim().split(/\s+/)[0];
        const actual = require("crypto").createHash("sha256").update(bytes).digest("hex");
        if (expected && expected !== actual) {
          throw new Error(`checksum mismatch (expected ${expected}, got ${actual})`);
        }
      }
    } catch (error) {
      if (String(error.message).includes("checksum mismatch")) {
        throw error;
      }
      note(`could not verify checksum: ${error.message}`);
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
      "Falling back to a `bitcode` on PATH. To install one:\n" +
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
