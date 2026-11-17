"""Bounded observations of a saved Ubuntu project. Never executes course scripts."""
import hashlib
import json
import os
from pathlib import Path
import platform
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import time

FILE_LIMIT = 64 * 1024 * 1024
TOOL_LIMIT = 128 * 1024 * 1024
TOOLS = {
    'cc': ['--version'], 'gcc': ['--version'], 'make': ['--version'],
    'cmake': ['--version'], 'git': ['--version'], 'python3': ['--version'],
    'dtc': ['--version'], 'arm-linux-gnueabihf-gcc': ['--version'],
    'aarch64-linux-gnu-gcc': ['--version'], 'ffmpeg': ['-version'],
    'ffprobe': ['-version'], 'pkg-config': ['--version'],
}


class ProbeError(Exception):
    pass


def environment():
    # Project/local PATH and Git process controls cannot redirect observations.
    env = {k: v for k, v in os.environ.items() if not k.startswith('GIT_')}
    env.update(PATH='/usr/bin:/bin', LC_ALL='C', GIT_OPTIONAL_LOCKS='0',
               GIT_TERMINAL_PROMPT='0', GIT_CONFIG_NOSYSTEM='1',
               GIT_CONFIG_GLOBAL='/dev/null', GIT_ATTR_NOSYSTEM='1')
    return env


def command(argv, root, deadline, limit=16384):
    if time.monotonic() >= deadline:
        raise ProbeError('TIME_BUDGET')
    process = subprocess.Popen(argv, cwd=root, env=environment(), stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
    stream = selectors.DefaultSelector()
    stream.register(process.stdout, selectors.EVENT_READ)
    data = bytearray()
    end = min(deadline, time.monotonic() + 3)
    completed = False
    try:
        while True:
            remaining = end - time.monotonic()
            if remaining <= 0 or not stream.select(remaining):
                raise ProbeError('PROCESS_TIMEOUT')
            chunk = os.read(process.stdout.fileno(), 8192)
            if not chunk:
                break
            data.extend(chunk)
            if len(data) > limit:
                raise ProbeError('OUTPUT_LIMIT')
        code = process.wait(timeout=max(0.01, end-time.monotonic()))
        completed = True
        return code, bytes(data)
    finally:
        stream.close()
        # Also end descendants holding the capture pipe after the parent exits.
        if not completed:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        process.wait()
        process.stdout.close()


def hash_fd(fd, limit, deadline):
    before = os.fstat(fd)
    if not stat.S_ISREG(before.st_mode) or before.st_size > limit:
        raise ProbeError('FILE_TYPE_OR_SIZE')
    digest = hashlib.sha256()
    total = 0
    while True:
        if time.monotonic() >= deadline:
            raise ProbeError('TIME_BUDGET')
        data = os.read(fd, 1024*1024)
        if not data:
            break
        total += len(data)
        if total > limit:
            raise ProbeError('FILE_SIZE')
        digest.update(data)
    after = os.fstat(fd)
    if (before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
            after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns) or total != after.st_size:
        raise ProbeError('FILE_CHANGED')
    return digest.hexdigest()


def file_digest(root, relative, deadline):
    parts = relative.split('/')
    if not relative or relative.startswith('/') or any(p in ('', '.', '..') for p in parts):
        raise ProbeError('PATH_INVALID')
    parent = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in parts[:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
            os.close(parent)
            parent = child
        fd = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        try:
            return hash_fd(fd, FILE_LIMIT, deadline)
        finally:
            os.close(fd)
    finally:
        os.close(parent)


def source(root, deadline):
    value = dict(kind='unavailable', commit='', branch='', dirty=None, untracked=None,
                 status='', message='')
    git = shutil.which('git', path='/usr/bin:/bin')
    if not git:
        value['message'] = 'GIT_MISSING'
        return value
    args = [git, '--no-optional-locks', '-c', 'core.fsmonitor=false',
            '-c', 'core.untrackedCache=false', '-c', 'core.hooksPath=/dev/null']
    try:
        code, data = command(args + ['rev-parse', '--is-inside-work-tree'], root, deadline)
        if code:
            if b'not a git repository' in data:
                value.update(kind='directory', message='NO_GIT_METADATA')
                return value
            raise ProbeError('GIT_UNAVAILABLE')
        # Disable every configured external conversion filter before status.
        code, data = command(args + ['config', '--null', '--get-regexp', r'^filter\..*\.(clean|smudge|process)$'], root, deadline)
        if code not in (0, 1):
            raise ProbeError('GIT_CONFIG_UNAVAILABLE')
        for entry in data.split(b'\0'):
            if not entry:
                continue
            key = entry.split(b'\n', 1)[0].decode('utf-8', 'strict')
            args += ['-c', key+'=', '-c', key.rsplit('.',1)[0]+'.required=false']
        code, commit = command(args + ['rev-parse', '--verify', 'HEAD'], root, deadline)
        if code == 0:
            commit = commit.decode().strip()
            if len(commit) not in (40,64) or any(c not in '0123456789abcdef' for c in commit):
                raise ProbeError('GIT_COMMIT_INVALID')
            value.update(kind='git', commit=commit)
        else:
            value.update(kind='unborn', message='NO_COMMIT')
        code, branch = command(args + ['symbolic-ref', '--quiet', '--short', 'HEAD'], root, deadline)
        if code not in (0,1):
            raise ProbeError('GIT_BRANCH_UNAVAILABLE')
        value['branch'] = branch.decode('utf-8','replace').strip() if code == 0 else '(detached)'
        code, status = command(args + ['status','--porcelain','--untracked-files=normal','--ignore-submodules=all'], root, deadline)
        if code:
            raise ProbeError('GIT_STATUS_UNAVAILABLE')
        value.update(dirty=bool(status), untracked=any(line.startswith(b'??') for line in status.splitlines()),
                     status=status.decode('utf-8','replace'))
    except (ProbeError, OSError, UnicodeError, subprocess.SubprocessError) as error:
        value.update(kind='unavailable', dirty=None, untracked=None, message=type(error).__name__)
    return value


def tool(name, root, deadline):
    value = dict(name=name, status='UNVERIFIED', path='', version='', sha256='', message='')
    if name not in TOOLS:
        value['message'] = 'TOOL_NOT_ALLOWED'
        return value
    path = shutil.which(name, path='/usr/bin:/bin')
    if not path:
        value.update(status='MISSING', message='TOOL_MISSING')
        return value
    try:
        path = str(Path(path).resolve(strict=True))
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            digest = hash_fd(fd, TOOL_LIMIT, deadline)
            before = os.fstat(fd)
            code, data = command([path, *TOOLS[name]], root, deadline)
            after = os.stat(path)
            if (before.st_dev,before.st_ino,before.st_size,before.st_mtime_ns,before.st_ctime_ns) != (
                    after.st_dev,after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns):
                raise ProbeError('TOOL_CHANGED')
        finally:
            os.close(fd)
        lines=data.decode('utf-8','replace').splitlines()
        if code or not lines:
            raise ProbeError('VERSION_UNAVAILABLE')
        value.update(status='OBSERVED', path=path, version=lines[0][:512], sha256=digest)
    except (ProbeError, OSError, subprocess.SubprocessError) as error:
        value['message'] = type(error).__name__
    return value


def inspect(root, requirements):
    root = Path(root).resolve(strict=True)
    if not root.is_dir() or len(requirements)>16:
        raise ProbeError('PROJECT_INVALID')
    deadline = time.monotonic() + 15
    os_name=platform.system()
    try:
        with open('/etc/os-release') as f:
            for line in f:
                if line.startswith('PRETTY_NAME='):
                    os_name=line.strip().split('=',1)[1].strip('"')[:512]
    except OSError:
        pass
    host=dict(os=os_name, architecture=platform.machine())
    git=source(root,deadline)
    names=['git','python3','cc']
    for check in requirements:
        if check['kind']=='tool' and check['selector'] not in names:
            names.append(check['selector'])
    tools={name:tool(name,root,deadline) for name in names}
    checks=[]
    for check in requirements:
        value={'spec':check,'observed':'','status':'UNVERIFIED','message':''}
        try:
            kind=check['kind'];operator=check['operator'];expected=check['expected']
            if kind=='tool':
                fact=tools[check['selector']]
                if fact['status']!='OBSERVED':
                    value.update(status=fact['status'],message=fact['message']);checks.append(value);continue
                observed=fact['sha256'] if operator=='sha256' else fact['version']
            elif kind=='file':
                observed=file_digest(root,check['selector'],deadline)
            elif kind in ('os','architecture'):
                observed=host[kind]
            elif kind=='git' and git['kind']=='git':
                observed=git['commit']
            else:
                raise ProbeError('OBSERVATION_UNAVAILABLE')
            value.update(observed=observed,status='MATCH' if operator=='present' or observed==expected else 'MISMATCH')
        except FileNotFoundError:
            value.update(status='MISSING',message='FILE_MISSING')
        except (ProbeError,OSError,KeyError) as error:
            value['message']=type(error).__name__
        checks.append(value)
    return dict(source=git,host=host,tools=list(tools.values()),checks=checks)


if __name__=='__main__':
    print(json.dumps(inspect(Path.cwd(),json.loads(sys.argv[1])),ensure_ascii=False,separators=(',',':')))
