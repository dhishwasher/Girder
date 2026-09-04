"""End-to-end tests for `install.sh`, the primary documented install path.

The README's first code block is `curl … | sh`, so this script is how most
people will ever obtain Bit Code, and nothing exercised it. These tests serve
a synthetic release from localhost and run the real script against it, which
covers the parts that only exist at runtime: the checksum gate, and the
promise that a failed install leaves no binary behind.

`BITCODE_BASE_URL` is what makes this possible without touching the network —
it exists so downloads can be pointed at a mirror, and a local HTTP server is
a mirror.
"""

import hashlib
import io
import os
import platform
import shutil
import subprocess
import tarfile
import tempfile
import threading
import unittest
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

INSTALL_SH = Path(__file__).parents[1] / "install.sh"
VERSION = "v9.9.9"
# What the fake binary prints, so the final `bitcode --version` line proves the
# thing that got installed is the thing that was served.
VERSION_OUTPUT = "bitcode 9.9.9"


def host_target() -> str | None:
    """The release asset triple `install.sh` will derive from this host.

    Returns None where the script is expected to refuse before downloading, so
    the tests skip rather than assert against an error they did not set up.
    """
    system = platform.system()
    machine = platform.machine().lower()
    if system == "Linux":
        os_part = "unknown-linux-gnu"
    elif system == "Darwin":
        os_part = "apple-darwin"
    else:
        return None
    if machine in ("x86_64", "amd64"):
        arch_part = "x86_64"
    elif machine in ("arm64", "aarch64"):
        arch_part = "aarch64"
    else:
        return None
    # install.sh refuses this pair on purpose: no such asset is published.
    if os_part == "unknown-linux-gnu" and arch_part == "aarch64":
        return None
    return f"{arch_part}-{os_part}"


class QuietHandler(SimpleHTTPRequestHandler):
    def log_message(self, *args):  # noqa: D102 - silence per-request logging
        pass


def tarball(members: dict[str, tuple[str, int]]) -> bytes:
    """A .tar.gz of `name -> (contents, mode)`, built in memory."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
        for name, (contents, mode) in members.items():
            payload = contents.encode()
            info = tarfile.TarInfo(name)
            info.size = len(payload)
            info.mode = mode
            archive.addfile(info, io.BytesIO(payload))
    return buffer.getvalue()


def release_archive() -> bytes:
    """A stand-in for a published asset: an executable `bitcode` plus its license.

    The binary is a shell script because install.sh runs `bitcode --version`
    to report what it installed, so the artifact has to actually execute.
    """
    return tarball(
        {
            "bitcode": (f"#!/bin/sh\necho '{VERSION_OUTPUT}'\n", 0o755),
            "LICENSE": ("Business Source License 1.1", 0o644),
        }
    )


@unittest.skipIf(host_target() is None, "no published asset for this host")
@unittest.skipUnless(
    shutil.which("curl") or shutil.which("wget"), "install.sh requires curl or wget"
)
class InstallShTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="bitcode-install-test-"))
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.asset = f"bitcode-{host_target()}.tar.gz"
        # The layout install.sh expects: <base>/<version>/<asset>.
        self.served = self.root / "served" / VERSION
        self.served.mkdir(parents=True)
        self.bin_dir = self.root / "bin"

        self.server = ThreadingHTTPServer(
            ("127.0.0.1", 0),
            partial(QuietHandler, directory=str(self.root / "served")),
        )
        thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(thread.join)
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.base_url = f"http://127.0.0.1:{self.server.server_address[1]}"

    def publish(self, archive: bytes | None = None, checksum: str | None = "match"):
        """Put an asset, and optionally a .sha256, where install.sh will look.

        `checksum="match"` publishes the real digest; any other string is
        published verbatim, and None publishes no checksum file at all.
        """
        if archive is None:
            archive = release_archive()
        (self.served / self.asset).write_bytes(archive)
        if checksum is None:
            return
        if checksum == "match":
            digest = hashlib.sha256(archive).hexdigest()
            checksum = f"{digest}  {self.asset}\n"
        (self.served / f"{self.asset}.sha256").write_text(checksum)

    def install(self, **env_overrides) -> subprocess.CompletedProcess:
        env = {
            **os.environ,
            "BITCODE_BASE_URL": self.base_url,
            "BITCODE_VERSION": VERSION,
            "BITCODE_BIN_DIR": str(self.bin_dir),
            "HOME": str(self.root),
        }
        env.pop("BITCODE_SKIP_CHECKSUM", None)
        env.update(env_overrides)
        return subprocess.run(
            ["sh", str(INSTALL_SH)],
            capture_output=True,
            text=True,
            env=env,
            timeout=120,
        )

    def assertNothingInstalled(self, result):
        self.assertNotEqual(result.returncode, 0, f"expected failure: {result.stderr}")
        self.assertFalse(
            (self.bin_dir / "bitcode").exists(),
            f"a failed install left a binary behind: {result.stderr}",
        )
        # The staging name is an implementation detail of the atomic install,
        # but leaving one behind would mean a half-written file survived.
        self.assertFalse((self.bin_dir / ".bitcode.incoming").exists(), result.stderr)

    def test_installs_a_verified_release(self):
        self.publish()
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        installed = self.bin_dir / "bitcode"
        self.assertTrue(installed.exists(), result.stderr)
        self.assertTrue(os.access(installed, os.X_OK), "installed binary is not executable")
        self.assertIn(VERSION_OUTPUT, result.stderr)
        # Nothing may report a skipped verification on the happy path, or the
        # gate below could pass while never actually checking anything.
        self.assertNotIn("not verified", result.stderr)

    def test_a_missing_checksum_is_fatal(self):
        # The release publishes a .sha256 for every asset, so its absence is a
        # broken release or a blocked request. Both must stop the install
        # instead of downgrading it to unverified.
        self.publish(checksum=None)
        self.assertNothingInstalled(self.install())

    def test_a_mismatched_checksum_is_fatal(self):
        self.publish(checksum=f"{'0' * 64}  {self.asset}\n")
        result = self.install()
        self.assertNothingInstalled(result)
        self.assertIn("checksum mismatch", result.stderr)

    def test_a_checksum_file_that_is_not_a_checksum_is_reported_as_malformed(self):
        # What a mirror answering 200 with an error page looks like. Calling
        # this a mismatch would point the user at their download instead of
        # at their mirror.
        self.publish(checksum="<html><body>404 Not Found</body></html>\n")
        result = self.install()
        self.assertNothingInstalled(result)
        self.assertIn("malformed checksum", result.stderr)
        self.assertNotIn("checksum mismatch", result.stderr)

    def test_skip_checksum_is_the_only_way_to_install_unverified(self):
        self.publish(checksum=None)
        result = self.install(BITCODE_SKIP_CHECKSUM="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.bin_dir / "bitcode").exists(), result.stderr)
        self.assertIn("not verified", result.stderr)

    def test_a_failed_install_does_not_disturb_an_existing_binary(self):
        # The reason the script stages under a temporary name: a working
        # binary already on PATH must survive a bad download.
        self.bin_dir.mkdir(parents=True)
        existing = self.bin_dir / "bitcode"
        existing.write_text("#!/bin/sh\necho 'bitcode 0.0.1'\n")
        existing.chmod(0o755)

        self.publish(checksum=f"{'0' * 64}  {self.asset}\n")
        result = self.install()
        self.assertNotEqual(result.returncode, 0, result.stderr)
        self.assertEqual(existing.read_text(), "#!/bin/sh\necho 'bitcode 0.0.1'\n")

    def test_an_archive_without_a_binary_is_fatal(self):
        self.publish(archive=tarball({"LICENSE": ("Business Source License 1.1", 0o644)}))
        result = self.install()
        self.assertNothingInstalled(result)
        self.assertIn("bitcode binary", result.stderr)

    def test_a_missing_asset_is_fatal(self):
        result = self.install()
        self.assertNothingInstalled(result)
        self.assertIn("download failed", result.stderr)


if __name__ == "__main__":
    unittest.main()
