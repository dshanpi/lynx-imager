"""Fixed read-only probe for explicitly selected, tracked teaching source files."""
import json
import os
from pathlib import Path
import platform
import select
import signal
import subprocess
import sys
import time

SUFFIXES = {".c", ".h", ".cc", ".cpp", ".hpp", ".rs", ".go", ".py", ".ts", ".tsx", ".js", ".jsx", ".sh"}


def source_path(value):
    if not isinstance(value, str) or not value or len(value) > 256:
        raise ValueError("invalid source path")
    parts = value.split("/")
    if any(not part or part.startswith(".") for part in parts):
        raise ValueError("hidden, absolute or traversal path")
    if any(ord(c) < 32 or c in "\\:" for c in value) or Path(value).suffix.lower() not in SUFFIXES:
        raise ValueError("unsupported source path")
    return value


def capture(argv, limit=32768, overall_deadline=None):
    process = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, start_new_session=True)
    output = bytearray()
    deadline = min(time.monotonic() + 10, overall_deadline or float("inf"))
    try:
        while True:
            left = deadline - time.monotonic()
            if left <= 0 or not select.select([process.stdout], [], [], left)[0]:
                raise ValueError("probe timeout")
            part = os.read(process.stdout.fileno(), 4096)
            if not part:
                break
            output.extend(part)
            if len(output) > limit:
                raise ValueError("probe output limit")
        if process.wait(timeout=max(0.01, deadline - time.monotonic())):
            raise ValueError("probe command failed")
        return output.decode("utf-8", "replace")
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        process.stdout.close()


def inspect(request):
    deadline = time.monotonic() + 15
    def read(argv, limit=32768):
        return capture(argv, limit, deadline)

    paths = request["sourcePaths"]
    if not isinstance(paths, list) or not 1 <= len(paths) <= 8 or len(set(paths)) != len(paths):
        raise ValueError("select 1 to 8 distinct source files")
    paths = [source_path(p) for p in paths]
    git = ["git", "--no-optional-locks", "--literal-pathspecs", "-c", "core.fsmonitor=false"]
    root = read(git + ["rev-parse", "--show-toplevel"]).strip()
    if Path(root).resolve() != Path.cwd().resolve():
        raise ValueError("workspace must be repository root")
    commit = read(git + ["rev-parse", "--verify", "HEAD"]).strip()
    if commit != request["commit"]:
        raise ValueError("signed source commit mismatch")
    for name in paths:
        item = Path(name)
        if item.is_symlink() or any(p.is_symlink() for p in item.parents):
            raise ValueError("symlink source forbidden")
        read(git + ["ls-files", "--error-unmatch", "--", name])
        # Newly staged files must not expand the signed source resource scope.
        if read(git + ["cat-file", "-t", f"HEAD:{name}"], 32).strip() != "blob" or item.is_dir():
            raise ValueError("selection must be a source file")
    diff = read(git + ["diff", "--no-ext-diff", "--no-textconv", "--unified=3", "HEAD", "--", *paths])
    if read(git + ["rev-parse", "--verify", "HEAD"]).strip() != commit:
        raise ValueError("source changed during inspection")
    return {"commit": commit, "branch": read(git + ["branch", "--show-current"]).strip(),
            "os": platform.system(), "architecture": platform.machine(),
            "compiler": read(["cc", "--version"], 8192).splitlines()[0],
            "sourcePaths": paths, "diff": diff, "untrackedIncluded": False}


if __name__ == "__main__":
    try:
        request = json.loads(sys.stdin.buffer.read(8193))
        print(json.dumps(inspect(request)))
    except Exception:
        # Never relay external command stderr or arbitrary paths/credentials.
        print(json.dumps({"error": "LEARN_TUTOR_PROBE_FAILED"}))
        sys.exit(1)
