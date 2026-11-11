import hashlib
import errno
import io
from unittest import mock
from types import SimpleNamespace
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import subprocess
import sys
import time
import unittest

spec = importlib.util.spec_from_file_location("helper", Path(__file__).parents[2] / "src/remoted/files_helper.py")
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)

class FilesTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.fs = helper.Files(str(self.root), False)
    def tearDown(self):
        self.fs.close()
        self.temp.cleanup()
    def upload(self, path, data, overwrite=False):
        return self.fs.run(dict(action="upload", path=path, size=len(data), sha256=hashlib.sha256(data).hexdigest(), overwrite=overwrite))["transfer"]
    def test_roundtrip_chunks_hash_and_no_clobber(self):
        data = os.urandom(100000)
        token = self.upload("sample", data)
        for offset in range(0, len(data), helper.CHUNK):
            self.fs.run(dict(action="chunk", transfer=token, offset=offset, data=helper.encode(data[offset:offset+helper.CHUNK])))
        self.fs.run(dict(action="commit", transfer=token))
        token = self.fs.run(dict(action="download", path="sample"))["transfer"]
        received = b""
        while True:
            response = self.fs.run(dict(action="read", transfer=token, offset=len(received)))
            if response.get("done"):
                self.assertEqual(response["sha256"], hashlib.sha256(data).hexdigest())
                break
            received += helper.decode(response["data"])
        self.assertEqual(received, data)
        duplicate = self.upload("sample", b"")
        with self.assertRaises(FileExistsError): self.fs.run(dict(action="commit", transfer=duplicate))
        self.assertEqual((self.root / "sample").read_bytes(), data)
    def test_upload_rejects_cross_filesystem_before_creating_transfer(self):
        self.fs.stage = helper.staging(self.fs.root)
        original = os.fstat
        def different_device(fd):
            info = original(fd)
            if fd == self.fs.stage:
                return SimpleNamespace(st_dev=info.st_dev + 1)
            return info
        with mock.patch.object(helper.os, "fstat", side_effect=different_device):
            with self.assertRaises(OSError) as caught:
                self.upload("cross-mount", b"pending")
        self.assertEqual(caught.exception.errno, errno.EXDEV)
        self.assertEqual(self.fs.uploads, {})
        self.assertEqual(list((self.root / helper.STAGING).iterdir()), [])
        self.assertFalse((self.root / "cross-mount").exists())
        # Failed begin releases the parent fd and leaves the helper reusable.
        self.upload("same-mount", b"")
    def test_upload_rejects_same_device_bind_mount_boundary(self):
        self.fs.stage = helper.staging(self.fs.root)
        # A bind mount has the same st_dev but a distinct mount ID.
        self.assertEqual(os.fstat(self.fs.root).st_dev, os.fstat(self.fs.stage).st_dev)
        def fdinfo(path, **kwargs):
            fd = int(path.rsplit("/", 1)[1])
            return io.StringIO("pos:\t0\nflags:\t0100000\nmnt_id:\t" + ("18" if fd == self.fs.stage else "17") + "\n")
        before = set(os.listdir("/proc/self/fd"))
        with mock.patch.object(helper, "open", side_effect=fdinfo, create=True):
            with self.assertRaises(OSError) as caught:
                self.upload("bind-mount", b"pending")
        self.assertEqual(caught.exception.errno, errno.EXDEV)
        self.assertEqual(self.fs.uploads, {})
        self.assertEqual(list((self.root / helper.STAGING).iterdir()), [])
        self.assertFalse((self.root / "bind-mount").exists())
        self.assertEqual(set(os.listdir("/proc/self/fd")), before)
        self.upload("same-mount", b"")
    def test_upload_fails_closed_when_mount_identity_unavailable(self):
        self.fs.stage = helper.staging(self.fs.root)
        for value in ("", "mnt_id:\tnot-a-number\n", "mnt_id:\t0\n", "mnt_id:\t17\nmnt_id:\t18\n", "x" * 4097):
            with self.subTest(value=value[:40]):
                with mock.patch.object(helper, "open", mock.mock_open(read_data=value), create=True):
                    with self.assertRaises(OSError) as caught:
                        self.upload("unverified", b"pending")
                self.assertEqual(caught.exception.errno, errno.ENOTSUP)
        with mock.patch.object(helper, "open", side_effect=PermissionError(), create=True):
            with self.assertRaises(OSError) as caught:
                self.upload("unreadable", b"pending")
        self.assertEqual(caught.exception.errno, errno.ENOTSUP)
        self.assertEqual(self.fs.uploads, {})
        self.assertEqual(list((self.root / helper.STAGING).iterdir()), [])
    def test_paths_and_symlink_escapes_are_rejected(self):
        for path in ("../escape", "/etc/passwd", "x/../escape", "bad\0name", helper.STAGING + "/private"):
            with self.assertRaises(ValueError): self.fs.run(dict(action="download", path=path))
        os.symlink("/tmp", self.root / "link")
        with self.assertRaises(OSError): self.fs.run(dict(action="upload", path="link/escape", size=0, sha256=hashlib.sha256(b"").hexdigest(), overwrite=False))
        os.symlink("/etc/passwd", self.root / "file-link")
        with self.assertRaises(OSError): self.fs.run(dict(action="download", path="file-link"))
    def test_readonly_and_cancel(self):
        readonly = helper.Files(str(self.root), True)
        try:
            with self.assertRaises(PermissionError): readonly.run(dict(action="upload"))
        finally: readonly.close()
        token = self.upload("never-committed", b"pending")
        self.fs.run(dict(action="cancel", transfer=token))
        self.assertFalse((self.root / "never-committed").exists())
        self.assertEqual(list((self.root / helper.STAGING).iterdir()), [])
    def test_corrupt_digest_and_wrong_offset_cannot_commit(self):
        token = self.upload("bad", b"abcd")
        with self.assertRaises(ValueError): self.fs.run(dict(action="chunk", transfer=token, offset=1, data=helper.encode(b"abcd")))
        self.fs.run(dict(action="chunk", transfer=token, offset=0, data=helper.encode(b"ABCD")))
        with self.assertRaises(ValueError): self.fs.run(dict(action="commit", transfer=token))
        self.assertFalse((self.root / "bad").exists())
    def test_staging_cannot_be_symlink_or_world_readable(self):
        os.mkdir(self.root / helper.STAGING, 0o755)
        with self.assertRaises(ValueError): self.upload("bad", b"")
    def test_explicit_overwrite_replaces_link_without_following_it(self):
        outside = self.root / "original"
        outside.write_bytes(b"keep")
        os.symlink("original", self.root / "destination")
        token = self.upload("destination", b"", overwrite=True)
        self.fs.run(dict(action="commit", transfer=token))
        self.assertEqual(outside.read_bytes(), b"keep")
        self.assertFalse((self.root / "destination").is_symlink())
    def test_directory_listing_is_bounded_after_json_escaping(self):
        for index in range(180):
            (self.root / (str(index) + "猫" * 70)).touch()
        result = self.fs.run(dict(action="list", path=""))
        self.assertTrue(result["truncated"])
        self.assertGreater(len(result["entries"]), 0)
        record = json.dumps({"ok": result}, ensure_ascii=True, separators=(",", ":")).encode() + b"\n"
        self.assertLessEqual(len(record), helper.MAX_LINE)
        event = json.dumps(dict(id=2**64-1, event=dict(type="file_list", path="猫" * 4096, **result)), ensure_ascii=False, separators=(",", ":")).encode()
        self.assertLess(len(event), 61440)
    def test_directory_listing_caps_count_and_hides_staging(self):
        for index in range(1001):
            (self.root / str(index)).touch()
        os.mkdir(self.root / helper.STAGING, 0o700)
        result = self.fs.run(dict(action="list", path="."))
        self.assertTrue(result["truncated"])
        self.assertLessEqual(len(result["entries"]), 1000)
        self.assertNotIn(helper.STAGING, [entry["name"] for entry in result["entries"]])
    def test_repeated_root_listing_uses_an_independent_directory_cursor(self):
        (self.root / "first").touch()
        self.assertEqual([entry["name"] for entry in self.fs.run(dict(action="list", path=""))["entries"]], ["first"])
        (self.root / "second").touch()
        self.assertEqual({entry["name"] for entry in self.fs.run(dict(action="list", path=""))["entries"]}, {"first", "second"})
    def test_staging_reaper_preserves_active_uploads_and_unrecognized_files(self):
        self.upload("active", b"")
        stage = self.root / helper.STAGING
        active = next(stage.iterdir())
        stale = stage / ("upload-" + "a" * 32 + ".part")
        stale.touch(mode=0o600)
        unknown = stage / "keep"
        unknown.touch(mode=0o600)
        old = time.time() - 90000
        for path in [active, stale, unknown]: os.utime(path, (old, old))
        other = helper.Files(str(self.root), False)
        try:
            other.run(dict(action="upload", path="second", size=0, sha256=hashlib.sha256(b"").hexdigest(), overwrite=False))
            self.assertTrue(active.exists())
            self.assertFalse(stale.exists())
            self.assertTrue(unknown.exists())
        finally: other.close()
    def test_transfer_limit_and_release(self):
        first = self.upload("first", b"")
        self.upload("second", b"")
        with self.assertRaises(ValueError): self.upload("third", b"")
        self.fs.run(dict(action="cancel", transfer=first))
        self.upload("third", b"")
    def test_download_detects_mutation_at_eof(self):
        path = self.root / "changing"
        path.write_bytes(b"first")
        token = self.fs.run(dict(action="download", path="changing"))["transfer"]
        self.fs.run(dict(action="read", transfer=token, offset=0))
        path.write_bytes(b"other" + b"bytes")
        with self.assertRaises(ValueError): self.fs.run(dict(action="read", transfer=token, offset=5))
    def test_fifo_download_never_blocks(self):
        os.mkfifo(self.root / "fifo")
        with self.assertRaises(ValueError): self.fs.run(dict(action="download", path="fifo"))
    def test_process_eof_cleans_uncommitted_staging(self):
        request = dict(action="upload", path="pending", size=0, sha256=hashlib.sha256(b"").hexdigest(), overwrite=False)
        result = subprocess.run([sys.executable, helper.__file__, str(self.root), "operate"], input=json.dumps(request) + "\n", capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        records = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertIn("ready", records[0])
        self.assertIn("transfer", records[1]["ok"])
        self.assertEqual(list((self.root / helper.STAGING).iterdir()), [])
        self.assertFalse((self.root / "pending").exists())

if __name__ == "__main__": unittest.main()
