"use strict";

// Unit tests for the npm wrapper's binary resolution.
//
// The case that matters most is the third one: `npm install -g bitcode-mcp`
// puts a `bitcode` symlink on PATH that points back into this package, and
// resolving to it means the shim spawns itself without bound.

const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const SOURCE = path.join(__dirname, "..");

/**
 * A throwaway copy of the package, so `__dirname`-relative resolution is
 * exercised against a directory layout the test controls. The `bin` scripts
 * are made executable because that is what npm does to `bin` targets on
 * install, and executability is what the PATH scan tests for.
 */
function makePackage() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "bitcode-npm-test-"));
  const pkg = path.join(root, "package");
  fs.mkdirSync(path.join(pkg, "bin"), { recursive: true });
  fs.copyFileSync(path.join(SOURCE, "resolve.js"), path.join(pkg, "resolve.js"));
  for (const script of ["bitcode.js", "bitcode-mcp.js"]) {
    const destination = path.join(pkg, "bin", script);
    fs.copyFileSync(path.join(SOURCE, "bin", script), destination);
    fs.chmodSync(destination, 0o755);
  }
  return { root, pkg };
}

function load(pkg) {
  const entry = path.join(pkg, "resolve.js");
  delete require.cache[require.resolve(entry)];
  return require(entry);
}

function scratchDir(root, name) {
  const dir = path.join(root, name);
  fs.mkdirSync(dir, { recursive: true });
  return dir;
}

/** An executable stub standing in for a real, native `bitcode`. */
function writeExecutable(dir, name) {
  const file = path.join(dir, name);
  fs.writeFileSync(file, "#!/bin/sh\nexit 0\n");
  fs.chmodSync(file, 0o755);
  return file;
}

function withEnv(overrides, body) {
  const saved = new Map();
  for (const [key, value] of Object.entries(overrides)) {
    saved.set(key, process.env[key]);
    if (value === undefined) {
      delete process.env[key];
    } else {
      process.env[key] = value;
    }
  }
  try {
    return body();
  } finally {
    for (const [key, value] of saved) {
      if (value === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  }
}

test("resolves nothing when PATH is empty and no binary was vendored", () => {
  const { pkg } = makePackage();
  const resolve = load(pkg);
  withEnv({ PATH: "", [resolve.REENTRY_ENV]: undefined }, () => {
    assert.strictEqual(resolve.resolveBinary(), null);
  });
});

test("prefers a real binary found on PATH", () => {
  const { root, pkg } = makePackage();
  const resolve = load(pkg);
  const dir = scratchDir(root, "usr-local-bin");
  const real = writeExecutable(dir, resolve.binaryName());
  withEnv({ PATH: dir, [resolve.REENTRY_ENV]: undefined }, () => {
    assert.strictEqual(resolve.resolveBinary(), real);
  });
});

test("a global-install symlink pointing back into this package is not resolved", () => {
  const { root, pkg } = makePackage();
  const resolve = load(pkg);
  // What `npm install -g bitcode-mcp` leaves behind: a `bitcode` entry in a
  // global bin directory that is really this package's own JS shim.
  const globalBin = scratchDir(root, "global-bin");
  fs.symlinkSync(
    path.join(pkg, "bin", "bitcode.js"),
    path.join(globalBin, resolve.binaryName())
  );
  withEnv({ PATH: globalBin, [resolve.REENTRY_ENV]: undefined }, () => {
    assert.strictEqual(
      resolve.resolveBinary(),
      null,
      "resolving to our own shim makes the shim spawn itself forever"
    );
  });
});

test("skips this package's own shim but still finds a real binary behind it", () => {
  const { root, pkg } = makePackage();
  const resolve = load(pkg);
  const globalBin = scratchDir(root, "global-bin");
  fs.symlinkSync(
    path.join(pkg, "bin", "bitcode.js"),
    path.join(globalBin, resolve.binaryName())
  );
  const realDir = scratchDir(root, "cargo-bin");
  const real = writeExecutable(realDir, resolve.binaryName());
  const search = [globalBin, realDir].join(path.delimiter);
  withEnv({ PATH: search, [resolve.REENTRY_ENV]: undefined }, () => {
    assert.strictEqual(resolve.resolveBinary(), real);
  });
});

test("a shim launched by another shim ignores PATH and uses the vendored binary", () => {
  const { root, pkg } = makePackage();
  const resolve = load(pkg);
  const vendored = writeExecutable(path.join(pkg, "bin"), resolve.binaryName());
  const dir = scratchDir(root, "usr-local-bin");
  writeExecutable(dir, resolve.binaryName());
  withEnv({ PATH: dir, [resolve.REENTRY_ENV]: "1" }, () => {
    assert.strictEqual(resolve.resolveBinary(), vendored);
  });
});

test("childEnv marks the chain so the next shim cannot loop", () => {
  const { pkg } = makePackage();
  const resolve = load(pkg);
  withEnv({ [resolve.REENTRY_ENV]: undefined }, () => {
    assert.strictEqual(resolve.childEnv()[resolve.REENTRY_ENV], "1");
  });
});

test("BITCODE_FORCE_VENDORED=1 uses the vendored binary even with a real one on PATH", () => {
  const { root, pkg } = makePackage();
  const resolve = load(pkg);
  const vendored = writeExecutable(path.join(pkg, "bin"), resolve.binaryName());
  const dir = scratchDir(root, "usr-local-bin");
  writeExecutable(dir, resolve.binaryName());
  withEnv(
    { PATH: dir, [resolve.REENTRY_ENV]: undefined, [resolve.FORCE_VENDORED_ENV]: "1" },
    () => {
      assert.strictEqual(resolve.resolveBinary(), vendored);
    }
  );
});

test("BITCODE_FORCE_VENDORED=1 fails loudly naming the missing vendored path", () => {
  const { root, pkg } = makePackage();
  const resolve = load(pkg);
  const dir = scratchDir(root, "usr-local-bin");
  writeExecutable(dir, resolve.binaryName());
  const expected = resolve.vendoredPath();
  withEnv(
    { PATH: dir, [resolve.REENTRY_ENV]: undefined, [resolve.FORCE_VENDORED_ENV]: "1" },
    () => {
      assert.throws(
        () => resolve.resolveBinary(),
        (error) => error instanceof Error && error.message.includes(expected)
      );
    }
  );
});

test("resolutionNotice is silent about a second binary when only the vendored one exists", () => {
  const { pkg } = makePackage();
  const resolve = load(pkg);
  const vendored = writeExecutable(path.join(pkg, "bin"), resolve.binaryName());
  const lines = resolve.resolutionNotice("bitcode-mcp", vendored);
  assert.strictEqual(lines.length, 1);
  assert.match(lines[0], /downloaded by this package/);
});

test("resolutionNotice names both paths and both --version outputs when PATH wins over an existing vendored copy", () => {
  const { root, pkg } = makePackage();
  const resolve = load(pkg);
  const vendored = path.join(pkg, "bin", resolve.binaryName());
  fs.writeFileSync(vendored, '#!/bin/sh\necho "bitcode 0.1.1"\n');
  fs.chmodSync(vendored, 0o755);
  const dir = scratchDir(root, "cargo-bin");
  const onPath = path.join(dir, resolve.binaryName());
  fs.writeFileSync(onPath, '#!/bin/sh\necho "bitcode 0.1.0"\n');
  fs.chmodSync(onPath, 0o755);

  const lines = resolve.resolutionNotice("bitcode-mcp", onPath);
  assert.strictEqual(lines.length, 2);
  assert.match(lines[0], /found on PATH, which takes precedence/);
  assert.ok(lines[1].includes(onPath), "missing the PATH binary's path");
  assert.ok(lines[1].includes(vendored), "missing the vendored binary's path");
  assert.ok(lines[1].includes("bitcode 0.1.0"), "missing the PATH binary's --version output");
  assert.ok(lines[1].includes("bitcode 0.1.1"), "missing the vendored binary's --version output");
});
