"""Target-side fd-scoped transfer helper. Stdout is only bounded JSON records.

Run as the authorized target login, never as the provider. Root object identity
is reported before accepting operations. The caller must compare it to its grant.
"""
import base64
import fcntl
import errno
import hashlib
import json
import os
import re
import stat
import sys
import time
import uuid

MAX_LINE = 65536
LIST_BUDGET = 40 * 1024
CHUNK = 32768
MAX_FILE = 4 * 1024**3
NOFOLLOW = os.O_NOFOLLOW
DIRECTORY = os.O_DIRECTORY
STAGING = ".neko-remoted-v1"


def emit(value):
    line = json.dumps(value, ensure_ascii=True, separators=(",", ":"))
    if len(line) + 1 > MAX_LINE:
        raise ValueError("response exceeds protocol limit")
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


def parts(path):
    if not isinstance(path, str) or len(path) > 4096 or "\0" in path or path.startswith("/"):
        raise ValueError("use a relative path within the shared directory")
    result = [] if path in ("", ".") else path.split("/")
    if any(p in ("", ".", "..", STAGING) for p in result):
        raise ValueError("invalid path")
    return result


def directory(root, components):
    fd = os.open(".", os.O_RDONLY | DIRECTORY | NOFOLLOW, dir_fd=root)
    try:
        for component in components:
            child = os.open(component, os.O_RDONLY | DIRECTORY | NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
        return fd
    except BaseException:
        os.close(fd)
        raise


def parent(root, path):
    p = parts(path)
    if not p:
        raise ValueError("file name required")
    return directory(root, p[:-1]), p[-1]


def decode(data):
    if not isinstance(data, str) or len(data) > 43692 or not re.fullmatch(r"[A-Za-z0-9_-]*", data):
        raise ValueError("invalid chunk")
    result = base64.b64decode(data + "=" * (-len(data) % 4), altchars=b"-_", validate=True)
    if len(result) > CHUNK:
        raise ValueError("chunk too large")
    return result


def encode(data):
    return base64.urlsafe_b64encode(data).decode("ascii").rstrip("=")


def staging(root):
    try:
        os.mkdir(STAGING, 0o700, dir_fd=root)
    except FileExistsError:
        pass
    fd = os.open(STAGING, os.O_RDONLY | DIRECTORY | NOFOLLOW, dir_fd=root)
    info = os.fstat(fd)
    if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700:
        os.close(fd)
        raise ValueError("unsafe transfer staging directory")
    try:
        cleanup_staging(fd)
    except BaseException:
        os.close(fd)
        raise
    return fd


def cleanup_staging(fd):
    os.lseek(fd, 0, os.SEEK_SET)
    now = time.time()
    with os.scandir(fd) as entries:
        for i, entry in enumerate(entries):
            if i >= 4096:
                break
            if not re.fullmatch(r"upload-[a-f0-9]{32}\.part", entry.name):
                continue
            try:
                s = entry.stat(follow_symlinks=False)
                if not stat.S_ISREG(s.st_mode) or s.st_uid != os.geteuid() or stat.S_IMODE(s.st_mode) != 0o600 or now - s.st_mtime <= 86400:
                    continue
                candidate = os.open(entry.name, os.O_RDONLY | NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
                try:
                    fcntl.flock(candidate, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    opened = os.fstat(candidate)
                    if (opened.st_dev, opened.st_ino) == (s.st_dev, s.st_ino):
                        os.unlink(entry.name, dir_fd=fd)
                finally:
                    os.close(candidate)
            except (FileNotFoundError, BlockingIOError):
                continue


class Files:
    def __init__(self, root_path, read_only):
        if not os.path.isabs(root_path):
            raise ValueError("shared root must be absolute")
        self.root = os.open(root_path, os.O_RDONLY | DIRECTORY | NOFOLLOW)
        self.read_only = read_only
        self.uploads = {}
        self.downloads = {}
        self.stage = None
        self.last_cleanup = time.monotonic()

    def identity(self):
        s = os.fstat(self.root)
        return {"device": s.st_dev, "inode": s.st_ino, "uid": os.geteuid()}

    def run(self, req):
        if self.stage is not None and time.monotonic() - self.last_cleanup >= 300:
            cleanup_staging(self.stage)
            self.last_cleanup = time.monotonic()
        action = req.get("action")
        if action == "list":
            fd = directory(self.root, parts(req["path"]))
            try:
                result = []
                budget = LIST_BUDGET
                truncated = False
                with os.scandir(fd) as entries:
                    for e in entries:
                        if e.name == STAGING:
                            continue
                        if len(result) >= 1000:
                            truncated = True
                            break
                        s = e.stat(follow_symlinks=False)
                        # Refuse names that cannot be represented in the UTF-8 protocol.
                        e.name.encode("utf-8", "strict")
                        entry = {"name": e.name, "directory": stat.S_ISDIR(s.st_mode), "size": s.st_size}
                        cost = len(json.dumps(entry, ensure_ascii=True, separators=(",", ":"))) + 1
                        if cost > budget:
                            truncated = True
                            break
                        budget -= cost
                        result.append(entry)
                return {"entries": result, "truncated": truncated}
            finally:
                os.close(fd)
        if action == "download":
            if len(self.uploads) + len(self.downloads) >= 2:
                raise ValueError("transfer capacity reached")
            fd, name = parent(self.root, req["path"])
            try:
                source = os.open(name, os.O_RDONLY | NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
            finally:
                os.close(fd)
            try:
                info = os.fstat(source)
                if not stat.S_ISREG(info.st_mode) or info.st_size > MAX_FILE:
                    raise ValueError("not a supported regular file")
                token = uuid.uuid4().hex
                self.downloads[token] = {"fd": source, "size": info.st_size, "offset": 0, "hash": hashlib.sha256(), "identity": (info.st_size, info.st_mtime_ns, info.st_ctime_ns)}
                return {"transfer": token, "size": info.st_size}
            except BaseException:
                os.close(source)
                raise
        if action == "read":
            t = self.downloads[req["transfer"]]
            if req["offset"] != t["offset"]:
                raise ValueError("download offset mismatch")
            data = os.read(t["fd"], min(CHUNK, t["size"] - t["offset"]))
            if not data:
                info = os.fstat(t["fd"])
                if (info.st_size, info.st_mtime_ns, info.st_ctime_ns) != t["identity"] or t["offset"] != t["size"]:
                    raise ValueError("file changed during download")
                result = {"done": True, "size": t["size"], "sha256": t["hash"].hexdigest()}
                os.close(t["fd"])
                del self.downloads[req["transfer"]]
                return result
            offset = t["offset"]
            t["hash"].update(data)
            t["offset"] += len(data)
            return {"offset": offset, "data": encode(data)}
        if action in ("upload", "chunk", "commit") and self.read_only:
            raise PermissionError("device is read-only")
        if action == "upload":
            if len(self.uploads) + len(self.downloads) >= 2:
                raise ValueError("transfer capacity reached")
            size = req["size"]
            if type(size) is not int or not 0 <= size <= MAX_FILE or not re.fullmatch(r"[a-f0-9]{64}", req["sha256"]):
                raise ValueError("invalid file size or digest")
            if type(req["overwrite"]) is not bool:
                raise ValueError("overwrite must be explicit")
            fd, name = parent(self.root, req["path"])
            try:
                if self.stage is None:
                    self.stage = staging(self.root)
                if os.fstat(fd).st_dev != os.fstat(self.stage).st_dev:
                    raise OSError(errno.EXDEV, "upload destination and staging must share a filesystem")
                token = uuid.uuid4().hex
                tmp = "upload-" + token + ".part"
                f = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | NOFOLLOW, 0o600, dir_fd=self.stage)
                fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
                self.uploads[token] = {"fd": f, "parent": fd, "name": name, "temp": tmp, "size": size, "offset": 0, "expected": req["sha256"], "hash": hashlib.sha256(), "overwrite": req["overwrite"]}
                return {"transfer": token, "size": size}
            except BaseException:
                os.close(fd)
                raise
        if action == "chunk":
            t = self.uploads[req["transfer"]]
            data = decode(req["data"])
            if req["offset"] != t["offset"] or not data or t["offset"] + len(data) > t["size"]:
                raise ValueError("upload offset or size mismatch")
            remaining = memoryview(data)
            while remaining:
                n = os.write(t["fd"], remaining)
                if n <= 0:
                    raise OSError("short write")
                remaining = remaining[n:]
            t["hash"].update(data)
            t["offset"] += len(data)
            return {"offset": t["offset"]}
        if action == "commit":
            t = self.uploads[req["transfer"]]
            if t["offset"] != t["size"] or t["hash"].hexdigest() != t["expected"]:
                raise ValueError("file size or checksum mismatch")
            os.fsync(t["fd"])
            if t["overwrite"]:
                # Exact directory handle, explicit overwrite, atomic same-FS replacement.
                os.rename(t["temp"], t["name"], src_dir_fd=self.stage, dst_dir_fd=t["parent"])
            else:
                # Atomic no-clobber, no stat/rename race. Never follows a destination symlink.
                os.link(t["temp"], t["name"], src_dir_fd=self.stage, dst_dir_fd=t["parent"], follow_symlinks=False)
                os.unlink(t["temp"], dir_fd=self.stage)
            os.fsync(t["parent"])
            result = {"done": True, "size": t["size"], "sha256": t["expected"]}
            self.cancel(req["transfer"])
            return result
        if action == "cancel":
            self.cancel(req["transfer"])
            return {"cancelled": True}
        raise ValueError("unknown file operation")

    def cancel(self, token):
        t = self.uploads.pop(token, None)
        if t:
            os.close(t["fd"])
            os.close(t["parent"])
            try:
                os.unlink(t["temp"], dir_fd=self.stage)
            except FileNotFoundError:
                pass
        t = self.downloads.pop(token, None)
        if t:
            os.close(t["fd"])

    def close(self):
        for token in list(self.uploads) + list(self.downloads):
            self.cancel(token)
        if self.stage is not None:
            os.close(self.stage)
        os.close(self.root)


def main():
    fs = Files(sys.argv[1], sys.argv[2] == "read-only")
    try:
        emit({"ready": fs.identity()})
        while True:
            line = sys.stdin.buffer.readline(MAX_LINE + 1)
            if not line:
                break
            if len(line) > MAX_LINE or not line.endswith(b"\n"):
                raise ValueError("invalid request framing")
            try:
                req = json.loads(line)
                if not isinstance(req, dict):
                    raise ValueError("request must be an object")
                emit({"ok": fs.run(req)})
            except (ValueError, KeyError, OSError, UnicodeError) as error:
                # Deliberately omit OS paths and internal staging names from errors.
                emit({"error": type(error).__name__, "message": str(error) if isinstance(error, ValueError) else "file operation failed"})
    finally:
        fs.close()


if __name__ == "__main__":
    main()
