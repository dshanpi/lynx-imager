"""Fixed product runner transported over verified SSH. Never accepts shell text.

Task definitions come from .lynx/tasks.json at a course-pinned Git commit.
Results are local observations, not signed device/assessment authority.
"""
import contextlib
import base64
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import select
import shutil
import signal
import stat
import subprocess
import sys
import time

LIMIT = 1048576
ATOM = re.compile(r'^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$')
TERMINAL = {'succeeded', 'failed', 'cancelled', 'timed_out', 'output_limit', 'busy', 'unknown'}
TOOLS = {'make', 'cmake', 'ninja', 'cc', 'gcc', 'c++', 'g++', 'cargo', 'meson'}
SECRET_LINE = re.compile(
    r'''authorization\s*:\s*bearer\b|(?:api[_ -]?key|password|private[_ -]?key|(?:(?:access|refresh)[_ -]?)?token)["']?\s*[:=]|\bsk-[A-Za-z0-9_-]{8,}\b|https?://[^/\s:@]+:[^/\s@]+@''',
    re.IGNORECASE)


def safe_log_bytes(raw):
    # Inspect the whole bounded log before slicing: a PEM block or credential
    # assignment can begin before the displayed tail. Hash the original separately.
    lines = []
    private_key = False
    for line in raw.decode('utf-8', 'replace').splitlines(keepends=True):
        if '-----BEGIN' in line and 'PRIVATE KEY-----' in line:
            private_key = True
            lines.append('[REDACTED PRIVATE KEY]\n')
        if private_key:
            if '-----END' in line and 'PRIVATE KEY-----' in line:
                private_key = False
            continue
        lines.append(('[REDACTED CREDENTIAL LINE]' + ('\n' if line.endswith(('\n', '\r')) else '')) if SECRET_LINE.search(line) else line)
    return ''.join(lines).encode('utf-8')


def safe_log_tail(raw):
    return safe_log_bytes(raw)[-32768:].decode('utf-8', 'ignore')


def utc_now():
    return datetime.now(timezone.utc).isoformat(timespec='microseconds').replace('+00:00', 'Z')


def environment_facts():
    # Fixed, bounded host facts; no hostname, credentials, shell or guessed toolchain.
    os_name = ''
    try:
        with Path('/etc/os-release').open('rb') as file:
            raw = file.read(16385)
        if len(raw) > 16384:
            raise ValueError('ENVIRONMENT_FACTS_LIMIT')
        for line in raw.decode('utf-8', 'replace').splitlines():
            if line.startswith('PRETTY_NAME='):
                os_name = line.split('=', 1)[1].strip('"')
    except FileNotFoundError:
        pass
    facts = {'os': os_name, 'architecture': os.uname().machine,
             'pythonVersion': '.'.join(str(n) for n in sys.version_info[:3])}
    if any(len(v.encode()) > 512 or '\0' in v for v in facts.values()):
        raise ValueError('ENVIRONMENT_FACTS_LIMIT')
    return facts


def execution_context(task):
    value = {'collectorVersion': 'lynx-project-context/1',
             'sourceCommit': task['commit'], 'sourceFingerprint': task['learningSourceFingerprint'],
             'definitionSha256': task['definitionSha256'], 'facts': task['environmentFacts']}
    return {**value, 'fingerprint': digest(json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()), 'observedAt': utc_now()}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def atomic(path, value):
    temporary = path.with_suffix('.tmp')
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'wb') as file:
        file.write(encoded(value))
        file.flush()
        os.fsync(file.fileno())
    os.replace(temporary, path)


def read(path):
    with path.open('rb') as file:
        data = file.read(LIMIT + 1)
    if len(data) > LIMIT:
        raise ValueError('STATE_TOO_LARGE')
    return json.loads(data)


@contextlib.contextmanager
def lock(path, blocking=True):
    fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | (0 if blocking else fcntl.LOCK_NB))
        yield fd
    finally:
        os.close(fd)


def private_dir(path):
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    if path.is_symlink() or path.stat().st_uid != os.getuid() or path.stat().st_mode & 0o077:
        raise ValueError('STATE_DIRECTORY_NOT_PRIVATE')


def relative(root, name):
    if not isinstance(name, str) or not name or len(name) > 2048 or '\0' in name:
        raise ValueError('PATH_INVALID')
    part = Path(name)
    if part.is_absolute() or '..' in part.parts:
        raise ValueError('PATH_ESCAPE')
    path = (root / part).resolve()
    if path != root and root not in path.parents:
        raise ValueError('PATH_ESCAPE')
    return path


def git(root, *args):
    # Git inspection must not execute repository hooks, text converters or fsmonitor.
    p = subprocess.Popen(['git', '--no-optional-locks', '-c', 'core.fsmonitor=false', '-C', str(root), *args],
                         stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, start_new_session=True)
    data = bytearray()
    deadline = time.monotonic() + 15
    try:
        while True:
            left = deadline - time.monotonic()
            if left <= 0 or not select.select([p.stdout], [], [], left)[0]:
                raise ValueError('GIT_TIMEOUT')
            part = os.read(p.stdout.fileno(), 8192)
            if not part:
                break
            data.extend(part)
            if len(data) > 4 * LIMIT:
                raise ValueError('GIT_OUTPUT_LIMIT')
        if p.wait(timeout=max(.01, deadline - time.monotonic())) != 0:
            raise ValueError('GIT_FAILED')
        return bytes(data)
    finally:
        if p.poll() is None:
            os.killpg(p.pid, signal.SIGKILL)
        p.wait()
        p.stdout.close()


def system_executable(name, cwd):
    # Resolve PATH as the child would after chdir, then execute this exact path.
    # Observe bytes, never invoke an unreviewed --version command or shell.
    search = os.pathsep.join(str((cwd / part).resolve()) if not Path(part).is_absolute()
                             else part for part in os.get_exec_path())
    selected = shutil.which(name, path=search)
    if selected is None:
        raise ValueError('EXECUTABLE_UNAVAILABLE')
    try:
        path = Path(selected).resolve(strict=True)
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(fd, 'rb') as stream:
            before = os.fstat(stream.fileno())
            if not stat.S_ISREG(before.st_mode) or not os.access(path, os.X_OK):
                raise ValueError('EXECUTABLE_INVALID')
            if before.st_size > 512 * LIMIT:
                raise ValueError('EXECUTABLE_SIZE_LIMIT')
            h = hashlib.sha256()
            size = 0
            deadline = time.monotonic() + 15
            for part in iter(lambda: stream.read(65536), b''):
                size += len(part)
                if size > 512 * LIMIT or time.monotonic() > deadline:
                    raise ValueError('EXECUTABLE_READ_LIMIT')
                h.update(part)
            def identity(s):
                return (s.st_dev, s.st_ino, s.st_mode, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
            if (size != before.st_size or identity(before) != identity(os.fstat(stream.fileno()))
                    or identity(before) != identity(path.stat())
                    or Path(selected).resolve(strict=True) != path):
                raise ValueError('EXECUTABLE_CHANGED')
        return {'executablePath': str(path), 'executableSha256': h.hexdigest()}
    except (OSError, RuntimeError):
        raise ValueError('EXECUTABLE_UNAVAILABLE') from None


def definition(request):
    workspace = request['workspace']
    if not isinstance(workspace, str) or not Path(workspace).is_absolute():
        raise ValueError('WORKSPACE_INVALID')
    root = Path(workspace).resolve(strict=True)
    facts = environment_facts()
    for check in request.get('versionChecks', []):
        if check.get('component') != 'os' or check.get('operator') != 'eq':
            raise ValueError('ENVIRONMENT_CHECK_UNSUPPORTED')
        observed = facts['os']
        if observed != check.get('expected'):
            raise ValueError('ENVIRONMENT_VERSION_MISMATCH')
    commit = request['commit']
    if not re.fullmatch(r'[0-9a-f]{40}|[0-9a-f]{64}', commit):
        raise ValueError('COMMIT_INVALID')
    if git(root, 'rev-parse', '--show-toplevel').decode().strip() != str(root):
        raise ValueError('WORKSPACE_MUST_BE_REPOSITORY_ROOT')
    if git(root, 'rev-parse', '--verify', 'HEAD').decode().strip() != commit:
        raise ValueError('COMMIT_MISMATCH')
    raw = git(root, 'show', commit + ':.lynx/tasks.json')
    current = relative(root, '.lynx/tasks.json')
    if current.read_bytes() != raw:
        raise ValueError('TASK_DEFINITION_CHANGED')
    tasks = json.loads(raw)
    if tasks.get('schemaVersion') != 1 or not isinstance(tasks.get('tasks'), list) or len(tasks['tasks']) > 64:
        raise ValueError('TASK_SCHEMA_INVALID')
    matches = [t for t in tasks['tasks'] if t.get('id') == request['taskId']]
    if len(matches) != 1:
        raise ValueError('TASK_NOT_FOUND')
    task = matches[0]
    if set(task) - {'id', 'argv', 'cwd', 'timeoutSeconds', 'outputs'}:
        raise ValueError('TASK_SCHEMA_INVALID')
    argv = task.get('argv')
    if not isinstance(argv, list) or not 1 <= len(argv) <= 64 or any(
            not isinstance(a, str) or len(a) > 2048 or '\0' in a for a in argv):
        raise ValueError('ARGV_INVALID')
    cwd = relative(root, task.get('cwd', '.'))
    if argv[0] not in TOOLS:
        if not argv[0].startswith('./'):
            raise ValueError('EXECUTABLE_UNSUPPORTED')
        executable = relative(root, str(cwd.relative_to(root) / argv[0]))
        if not executable.is_file() or executable.stat().st_size > 512 * LIMIT:
            raise ValueError('EXECUTABLE_INVALID')
        executable_hash = hashlib.sha256()
        with executable.open('rb') as stream:
            for part in iter(lambda: stream.read(65536), b''):
                executable_hash.update(part)
        executable_identity = {'executableSha256': executable_hash.hexdigest()}
    else:
        executable_identity = system_executable(argv[0], cwd)
    timeout = task.get('timeoutSeconds')
    if type(timeout) is not int or not 1 <= timeout <= 3600:
        raise ValueError('TIMEOUT_INVALID')
    outputs = task.get('outputs')
    if not isinstance(outputs, list) or len(outputs) > 32 or len(set(outputs)) != len(outputs):
        raise ValueError('OUTPUTS_INVALID')
    for output in outputs:
        relative(root, output)
    generated = set()
    for other in tasks['tasks']:
        declared = other.get('outputs') if isinstance(other, dict) else None
        if not isinstance(declared, list) or len(declared) > 32:
            raise ValueError('OUTPUTS_INVALID')
        for name in declared:
            relative(root, name)
            generated.add(str(Path(name)))
    tracked = set(git(root, 'ls-files', '-z', '--cached').split(b'\0')) - {b''}
    files = sorted(set(git(root, 'ls-files', '-z', '--cached', '--others', '--exclude-standard').split(b'\0')) - {b''})
    if len(files) > 50000:
        raise ValueError('SOURCE_FILE_LIMIT')
    h = hashlib.sha256()
    learning_h = hashlib.sha256()
    total = 0
    for name in files:
        # Keep the original task-specific identity. The shared learning identity
        # also excludes untracked outputs from other reviewed tasks, but NEVER
        # hides tracked source or symlink identity behind an output declaration.
        text = os.fsdecode(name)
        file = root / text
        hashes = ([] if text in outputs else [h]) + ([] if text in generated and name not in tracked and not file.is_symlink() else [learning_h])
        if not hashes:
            continue
        def update(part):
            for target in hashes:
                target.update(part)
        update(name + b'\0')
        if file.is_symlink():
            update(b'link\0' + os.fsencode(os.readlink(file)))
        elif not file.exists():
            update(b'deleted\0')
        elif not file.is_file():
            raise ValueError('SOURCE_SUBMODULE_UNSUPPORTED')
        else:
            total += file.stat().st_size
            if total > 512 * LIMIT:
                raise ValueError('SOURCE_BYTES_LIMIT')
            update(str(file.stat().st_mode & 0o777).encode() + b'\0')
            with file.open('rb') as source:
                for part in iter(lambda: source.read(65536), b''):
                    update(part)
        update(b'\0')
    return {'workspace': str(root), 'commit': commit, 'taskId': request['taskId'],
            'argv': argv, 'cwd': str(cwd), 'timeoutSeconds': timeout, 'outputs': outputs,
            'definitionSha256': digest(raw), 'sourceFingerprint': h.hexdigest(), **executable_identity,
            'learningSourceFingerprint': learning_h.hexdigest(), 'environmentFacts': facts}


def public(job, status):
    result = {**status, 'runId': job.name}
    prepared = read(job / 'prepared.json')
    result['task'] = prepared['task']
    result['confirmation'] = prepared['confirmation']
    output = job / 'output.log'
    if output.exists():
        with output.open('rb') as file:
            raw = file.read(LIMIT + 1)
        if len(raw) > LIMIT:
            raise ValueError('LOG_SIZE_LIMIT')
        result['log'] = safe_log_tail(raw)
        result['logBytes'] = len(raw)
        result['logSha256'] = digest(raw)
    return result


def query(job):
    status = read(job / 'status.json')
    if status['state'] in ('starting', 'running', 'cancelling'):
        try:
            with lock(job / 'worker.lock', False):
                if time.time() - status['updatedAt'] > 10:
                    status = {**status, 'state': 'unknown', 'reason': 'WORKER_NOT_OBSERVABLE'}
        except BlockingIOError:
            pass
    return public(job, status)


def evidence(job, request, status):
    # Only fixed product outputs are readable. Legacy/non-process records do not
    # acquire invented timestamps or exit status during an export.
    if status.get('state') not in {'succeeded', 'failed', 'cancelled', 'timed_out', 'output_limit'} or type(status.get('exitCode')) is not int:
        raise ValueError('EVIDENCE_NOT_AVAILABLE')
    try:
        start = datetime.fromisoformat(status['startedAt'].replace('Z', '+00:00'))
        end = datetime.fromisoformat(status['finishedAt'].replace('Z', '+00:00'))
        if start.tzinfo is None or end.tzinfo is None or end < start:
            raise ValueError()
    except (KeyError, TypeError, AttributeError, ValueError):
        raise ValueError('EVIDENCE_NOT_AVAILABLE') from None
    kind = request.get('kind')
    if kind not in {'stdout', 'build_log', 'exit_status'}:
        raise ValueError('EVIDENCE_KIND_INVALID')
    if kind == 'exit_status':
        raw = encoded(status['exitCode'])
        clean = raw
    else:
        path = job / ('stdout.log' if kind == 'stdout' else 'output.log')
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            with os.fdopen(fd, 'rb') as file:
                if not stat.S_ISREG(os.fstat(file.fileno()).st_mode):
                    raise OSError('not a regular product output')
                raw = file.read(LIMIT + 1)
        except OSError:
            raise ValueError('EVIDENCE_NOT_AVAILABLE') from None
        if len(raw) > LIMIT:
            raise ValueError('LOG_SIZE_LIMIT')
        clean = safe_log_bytes(raw)
    if len(clean) > 4 * LIMIT:
        raise ValueError('EVIDENCE_SIZE_LIMIT')
    offset, length = request.get('offset'), request.get('length')
    if type(offset) is not int or type(length) is not int or not 0 <= offset <= len(clean) or not 1 <= length <= 49152:
        raise ValueError('EVIDENCE_RANGE_INVALID')
    expected = request.get('expectedSha256', '')
    expected_raw = request.get('expectedRawSha256', '')
    sha = digest(clean)
    raw_sha = digest(raw)
    if ((offset > 0 and (expected != sha or expected_raw != raw_sha)) or
            (expected and expected != sha) or (expected_raw and expected_raw != raw_sha)):
        raise ValueError('EVIDENCE_CHANGED')
    part = clean[offset:offset + length]
    return {'runId': job.name, 'kind': kind, 'state': status['state'],
            'startedAt': status['startedAt'], 'finishedAt': status['finishedAt'],
            'contentType': 'application/json' if kind == 'exit_status' else 'text/plain',
            'sha256': sha, 'size': len(clean), 'rawSha256': raw_sha, 'rawBytes': len(raw),
            'redacted': clean != raw, 'outputTruncated': status['state'] == 'output_limit',
            'collectorVersion': 'lynx-project-evidence/1',
            'offset': offset, 'nextOffset': offset + len(part), 'dataBase64': base64.b64encode(part).decode()}


def dispatch(request, root=None):
    root = Path(root) if root else Path.home() / '.local/state/lynx/learn-runs'
    private_dir(root)
    run = request.get('runId', '')
    scope = request.get('scope', '')
    if not ATOM.fullmatch(run) or not re.fullmatch('[a-f0-9]{64}', scope):
        raise ValueError('RUN_ID_INVALID')
    job = root / run
    operation = request['operation']
    if operation == 'prepare':
        # Prepare is read-only with respect to the project. Reusing an ID is never an overwrite.
        task = definition(request)
        private_dir(job)
        with lock(job / 'request.lock'):
            desired = {'request': request, 'task': task, 'confirmation': digest(encoded({'scope': scope, 'task': task}))}
            if (job / 'prepared.json').exists():
                if read(job / 'prepared.json') != desired:
                    raise ValueError('RUN_ID_CONFLICT')
            else:
                atomic(job / 'prepared.json', desired)
                atomic(job / 'status.json', {'state': 'prepared', 'updatedAt': time.time()})
        return query(job)
    if not job.is_dir() or job.is_symlink():
        raise ValueError('RUN_NOT_FOUND')
    with lock(job / 'request.lock'):
        prepared = read(job / 'prepared.json')
        if prepared['request']['scope'] != scope:
            raise ValueError('SCOPE_MISMATCH')
        status = read(job / 'status.json')
        if operation == 'inspect':
            if status['state'] not in ('succeeded', 'failed', 'cancelled', 'timed_out', 'output_limit') or not status.get('executionContext'):
                raise ValueError('RUN_NOT_FINISHED')
            current = definition(prepared['request'])
            return {'runId': run, 'scope': scope, 'executionContext': execution_context(current),
                    'executableSha256': current['executableSha256']}
        if operation == 'evidence':
            return evidence(job, request, status)
        if operation == 'start':
            if request.get('confirmation') != prepared['confirmation']:
                raise ValueError('CONFIRMATION_MISMATCH')
            if status['state'] != 'prepared':
                return query(job)  # Never replay a remotely accepted task.
            if definition(prepared['request']) != prepared['task']:
                raise ValueError('SOURCE_CHANGED')
            # Copy only fixed product code; no course data can become a script.
            runner = job / 'runner.py'
            fd = os.open(runner, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(fd, 'wb') as file:
                file.write(Path(__file__).read_bytes())
            atomic(job / 'status.json', {'state': 'starting', 'updatedAt': time.time()})
            child = subprocess.Popen([sys.executable, str(runner), '--worker', str(job)],
                             stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                             stderr=subprocess.DEVNULL, start_new_session=True, close_fds=True)
            # The worker intentionally outlives this SSH request; a short-lived
            # reaper thread avoids zombie children in long-lived test hosts.
            import threading
            threading.Thread(target=child.wait, daemon=True).start()
        elif operation == 'cancel':
            if status['state'] == 'prepared':
                atomic(job / 'status.json', {'state': 'cancelled', 'updatedAt': time.time()})
            elif status['state'] in ('starting', 'running', 'cancelling'):
                atomic(job / 'cancel.json', {'requestedAt': time.time()})
        elif operation != 'query':
            raise ValueError('OPERATION_INVALID')
        return query(job)


def worker(job):
    with lock(job / 'worker.lock', False):
        prepared = read(job / 'prepared.json')
        task = prepared['task']
        locks = Path.home() / '.local/state/lynx/project-locks'
        private_dir(locks)
        try:
            with lock(locks / (digest(os.fsencode(task['workspace'])) + '.lock'), False) as project_lock:
                if definition(prepared['request']) != task:
                    raise ValueError('SOURCE_CHANGED')
                execute(job, task, project_lock)
        except BlockingIOError:
            atomic(job / 'status.json', {'state': 'busy', 'reason': 'PROJECT_BUSY', 'updatedAt': time.time()})
        except Exception as error:
            # Fixed codes only. Exceptions may otherwise include source or local paths.
            reason = str(error) if isinstance(error, ValueError) and re.fullmatch('[A-Z_]+', str(error)) else 'RUNNER_FAILED'
            atomic(job / 'status.json', {'state': 'failed', 'reason': reason, 'updatedAt': time.time()})


def execute(job, task, project_lock):
    if (job / 'cancel.json').exists():
        atomic(job / 'status.json', {'state': 'cancelled', 'updatedAt': time.time()})
        return
    context = execution_context(task)
    started_at = utc_now()
    p = subprocess.Popen(task['argv'], executable=task.get('executablePath'), cwd=task['cwd'], stdin=subprocess.DEVNULL,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
                         pass_fds=(project_lock,))
    deadline = time.monotonic() + task['timeoutSeconds']
    state = 'running'
    count = 0
    streams = [p.stdout, p.stderr]
    atomic(job / 'status.json', {'state': state, 'startedAt': started_at, 'executionContext': context, 'updatedAt': time.time()})
    try:
        with (job / 'output.log').open('wb') as log, (job / 'stdout.log').open('wb') as stdout_log:
            while True:
                if (job / 'cancel.json').exists():
                    state = 'cancelled'
                    break
                if time.monotonic() >= deadline:
                    state = 'timed_out'
                    break
                if not streams:
                    # EOF is not process exit. Keep servicing cancellation and
                    # the deadline even when a long-running child closes output.
                    if p.poll() is not None:
                        break
                    time.sleep(.05)
                    continue
                for stream in select.select(streams, [], [], .05)[0]:
                    part = os.read(stream.fileno(), 8192)
                    if not part:
                        streams.remove(stream)
                        continue
                    bounded = part[:max(0, LIMIT - count)]
                    log.write(bounded)
                    log.flush()
                    if stream is p.stdout:
                        stdout_log.write(bounded)
                        stdout_log.flush()
                    count += len(part)
                    if count > LIMIT:
                        state = 'output_limit'
                        break
                if state == 'output_limit':
                    break
            if state == 'running':
                try:
                    p.wait(timeout=max(.01, deadline - time.monotonic()))
                    state = 'succeeded' if p.returncode == 0 else 'failed'
                except subprocess.TimeoutExpired:
                    state = 'timed_out'
    finally:
        # Stop surviving children even if their parent exited or closed stdout.
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        p.wait()
        p.stdout.close()
        p.stderr.close()
    finished_at = utc_now()
    artifacts = []
    if state == 'succeeded':
        for name in task['outputs']:
            file = relative(Path(task['workspace']), name)
            if not file.is_file():
                state = 'failed'
                break
            h = hashlib.sha256()
            with file.open('rb') as stream:
                for part in iter(lambda: stream.read(65536), b''):
                    h.update(part)
            artifacts.append({'path': name, 'sha256': h.hexdigest(), 'size': file.stat().st_size})
    atomic(job / 'status.json', {'state': state, 'exitCode': p.returncode, 'artifacts': artifacts,
                               'startedAt': started_at, 'finishedAt': finished_at, 'executionContext': context, 'updatedAt': time.time()})


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--worker':
        worker(Path(sys.argv[2]))
    else:
        try:
            raw = sys.stdin.buffer.read(65537)
            if len(raw) > 65536:
                raise ValueError('REQUEST_TOO_LARGE')
            print(json.dumps(dispatch(json.loads(raw))))
        except Exception as error:
            code = str(error) if isinstance(error, ValueError) and re.fullmatch('[A-Z_]+', str(error)) else 'RUNNER_REQUEST_FAILED'
            print(json.dumps({'error': code}))
            sys.exit(1)
