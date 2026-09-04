"use strict";

// End-to-end tests for the postinstall download.
//
// This is the path an agent host takes when it runs `npx girder-mcp`, so the
// bytes it lands are the bytes that end up executing. The cases that matter
// are the ones where verification cannot be completed: none of them may
// install anything, and none of them may fail the install either, because
// aborting a postinstall breaks `npx` with an error that names npm rather
// than the actual problem.

const test = require("node:test");
const assert = require("node:assert");
const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");
const { execFileSync, spawn } = require("node:child_process");

const SOURCE = path.join(__dirname, "..");
const VERSION = "v9.9.9";
const { target, binaryName } = require("../resolve");

// `tar -xf` is how install.js unpacks, and building the fixture needs a tar
// too. Both hold on the runners that execute this suite.
function haveTar() {
  try {
    execFileSync("tar", ["--version"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

const SUPPORTED = Boolean(target()) && haveTar();

/** A throwaway copy of the package, so the download lands somewhere disposable. */
function makePackage() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "girder-install-test-"));
  const pkg = path.join(root, "package");
  fs.mkdirSync(pkg, { recursive: true });
  for (const file of ["resolve.js", "install.js", "package.json"]) {
    fs.copyFileSync(path.join(SOURCE, file), path.join(pkg, file));
  }
  return { root, pkg };
}

/** A stand-in for a published asset: a .tar.gz holding an executable binary. */
function releaseArchive(root) {
  const staging = fs.mkdtempSync(path.join(root, "staging-"));
  const name = binaryName();
  fs.writeFileSync(path.join(staging, name), "#!/bin/sh\necho 'girder 9.9.9'\n");
  fs.chmodSync(path.join(staging, name), 0o755);
  const archive = path.join(root, "asset.tar.gz");
  execFileSync("tar", ["czf", archive, "-C", staging, name]);
  return fs.readFileSync(archive);
}

/**
 * Serve one asset, and optionally a checksum, at the paths install.js builds.
 *
 * `checksum` is the literal body to serve; null serves a 404 for it, and
 * "match" serves the real digest.
 */
function serve(asset, bytes, checksum) {
  const routes = new Map();
  // `bytes === null` publishes no asset at all, so the download itself 404s.
  if (bytes !== null) {
    routes.set(`/${VERSION}/${asset}`, { type: "application/octet-stream", body: bytes });
  }
  if (checksum !== null) {
    const body =
      checksum === "match"
        ? `${crypto.createHash("sha256").update(bytes).digest("hex")}  ${asset}\n`
        : checksum;
    routes.set(`/${VERSION}/${asset}.sha256`, { type: "text/plain", body });
  }
  const server = http.createServer((request, response) => {
    const route = routes.get(request.url);
    if (!route) {
      response.writeHead(404).end("not found");
      return;
    }
    response.writeHead(200, { "content-type": route.type }).end(route.body);
  });
  server.listen(0, "127.0.0.1");
  return server;
}

/**
 * Run the postinstall against `baseUrl` and collect its outcome.
 *
 * Asynchronous on purpose: the fixture server lives in this process, and a
 * synchronous spawn would block the event loop that has to answer the child's
 * requests — the child would wait for a response that cannot be sent.
 */
function run(pkg, baseUrl, env = {}) {
  const child = spawn(process.execPath, ["install.js"], {
    cwd: pkg,
    env: {
      ...process.env,
      GIRDER_BASE_URL: baseUrl,
      GIRDER_VERSION: VERSION,
      GIRDER_SKIP_CHECKSUM: "",
      ...env,
    },
  });
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  child.stdout.resume();
  return new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (status) => resolve({ status, stderr }));
  });
}

/** Set up a package and a server, run the postinstall, and report. */
async function postinstall({ checksum = "match", archive, env = {} } = {}) {
  const { root, pkg } = makePackage();
  const asset =
    os.platform() === "win32"
      ? `girder-${target()}.zip`
      : `girder-${target()}.tar.gz`;
  const bytes = archive === undefined ? releaseArchive(root) : archive;
  const server = serve(asset, bytes, checksum);
  await new Promise((resolve) => server.once("listening", resolve));
  try {
    const result = await run(pkg, `http://127.0.0.1:${server.address().port}`, env);
    return {
      result,
      installed: path.join(pkg, "bin", binaryName()),
      staging: path.join(pkg, "bin", `${binaryName()}.incoming`),
      cleanup: () => fs.rmSync(root, { recursive: true, force: true }),
    };
  } finally {
    server.close();
  }
}

/** A postinstall must never fail the install, whatever it decides to do. */
function assertCleanExit(result) {
  assert.strictEqual(result.status, 0, `postinstall exited ${result.status}: ${result.stderr}`);
}

test("a verified asset is installed", { skip: !SUPPORTED }, async () => {
  const { result, installed, cleanup } = await postinstall();
  assertCleanExit(result);
  assert.ok(fs.existsSync(installed), `nothing installed: ${result.stderr}`);
  fs.accessSync(installed, fs.constants.X_OK);
  assert.doesNotMatch(result.stderr, /not verified/);
  cleanup();
});

test("a missing checksum installs nothing", { skip: !SUPPORTED }, async () => {
  // The release publishes a .sha256 for every asset, so a 404 here means a
  // broken release or a blocked request rather than an old release.
  const { result, installed, staging, cleanup } = await postinstall({ checksum: null });
  assertCleanExit(result);
  assert.ok(!fs.existsSync(installed), "an unverifiable download was installed");
  assert.ok(!fs.existsSync(staging), "a partial write was left behind");
  assert.match(result.stderr, /no checksum at/);
  cleanup();
});

test("a mismatched checksum installs nothing", { skip: !SUPPORTED }, async () => {
  const { result, installed, cleanup } = await postinstall({
    checksum: `${"0".repeat(64)}  asset\n`,
  });
  assertCleanExit(result);
  assert.ok(!fs.existsSync(installed), "a mismatched download was installed");
  assert.match(result.stderr, /checksum mismatch/);
  cleanup();
});

test("a checksum file that is not a checksum is called malformed", { skip: !SUPPORTED }, async () => {
  const { result, installed, cleanup } = await postinstall({
    checksum: "<html><body>404 Not Found</body></html>\n",
  });
  assertCleanExit(result);
  assert.ok(!fs.existsSync(installed));
  assert.match(result.stderr, /malformed checksum/);
  assert.doesNotMatch(result.stderr, /checksum mismatch/);
  cleanup();
});

test("the skip flag is the only way to install unverified", { skip: !SUPPORTED }, async () => {
  const { result, installed, cleanup } = await postinstall({
    checksum: null,
    env: { GIRDER_SKIP_CHECKSUM: "1" },
  });
  assertCleanExit(result);
  assert.ok(fs.existsSync(installed), `nothing installed: ${result.stderr}`);
  assert.match(result.stderr, /not verified/);
  cleanup();
});

test("a failed download still falls back rather than failing", { skip: !SUPPORTED }, async () => {
  const { result, installed, cleanup } = await postinstall({ archive: null, checksum: null });
  assertCleanExit(result);
  assert.ok(!fs.existsSync(installed));
  assert.match(result.stderr, /Falling back to a `girder` on PATH/);
  cleanup();
});

test("GIRDER_SKIP_DOWNLOAD=1 downloads nothing", { skip: !SUPPORTED }, async () => {
  const { result, installed, cleanup } = await postinstall({
    env: { GIRDER_SKIP_DOWNLOAD: "1" },
  });
  assertCleanExit(result);
  assert.ok(!fs.existsSync(installed));
  assert.match(result.stderr, /not downloading/);
  cleanup();
});
