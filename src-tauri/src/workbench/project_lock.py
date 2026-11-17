"""Hold the same remote workspace lease used by Learn project tasks.

Only the existing user-saved Workbench command is accepted here. Learn does
not call this shell compatibility adapter.
"""
import fcntl
import hashlib
import os
from pathlib import Path
import sys

workspace = Path(sys.argv[1]).resolve(strict=True)
root = Path.home() / '.local/state/lynx/project-locks'
root.mkdir(parents=True, exist_ok=True, mode=0o700)
if root.is_symlink() or root.stat().st_uid != os.getuid() or root.stat().st_mode & 0o077:
    sys.exit('LYNX_PROJECT_LOCK_DIRECTORY_UNSAFE')
path = root / (hashlib.sha256(os.fsencode(str(workspace))).hexdigest() + '.lock')
fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
try:
    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
except BlockingIOError:
    sys.exit('LYNX_PROJECT_BUSY: another Workbench or Learn task owns this workspace')
os.set_inheritable(fd, True)
os.chdir(workspace)
os.execv('/bin/sh', ['sh', '-c', 'exec ' + sys.argv[2]])
