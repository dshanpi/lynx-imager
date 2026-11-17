"""Real process tests for the fixed Learn Ubuntu runner; no SSH or hardware."""
import importlib.util
import base64
from datetime import datetime
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
from unittest import mock

SOURCE = Path(__file__).resolve().parents[2] / 'src-tauri/src/learning/v2/project_runner.py'


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.repo = self.root / 'project'
        self.repo.mkdir()
        self.state = self.root / 'state'
        self.git('init', '-q')
        self.git('config', 'user.name', 'Learn test')
        self.git('config', 'user.email', 'learn@example.invalid')
        (self.repo / '.lynx').mkdir()
        (self.repo / '.gitignore').write_text('app\n')
        (self.repo / 'main.c').write_text('#include <stdio.h>\nint main(){puts("hello");return 0;}\n')
        (self.repo / '.lynx/tasks.json').write_text(json.dumps({'schemaVersion': 1, 'tasks': [
            {'id': 'build', 'argv': ['cc', 'main.c', '-o', 'app'], 'timeoutSeconds': 30, 'outputs': ['app']},
            {'id': 'run', 'argv': ['./app'], 'timeoutSeconds': 5, 'outputs': []},
            {'id': 'wait', 'argv': ['make', 'wait'], 'timeoutSeconds': 1, 'outputs': []},
            {'id': 'long', 'argv': ['make', 'wait'], 'timeoutSeconds': 30, 'outputs': []},
            {'id': 'flood', 'argv': ['make', 'flood'], 'timeoutSeconds': 10, 'outputs': []},
        ]}))
        (self.repo / 'Makefile').write_text('wait:\n\tsleep 10\nflood:\n\tyes x\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')
        self.commit = self.git('rev-parse', 'HEAD').strip()
        spec = importlib.util.spec_from_file_location('learn_runner', SOURCE)
        self.runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.runner)

    def tearDown(self):
        self.temp.cleanup()

    def git(self, *args):
        return subprocess.check_output(['git', '-C', str(self.repo), *args], text=True)

    def prepare(self, task='build', run='run_123'):
        return self.runner.dispatch({'operation': 'prepare', 'runId': run, 'workspace': str(self.repo),
                                     'commit': self.commit, 'taskId': task, 'scope': 'a' * 64}, self.state)

    def operation(self, operation, run='run_123', **extra):
        return self.runner.dispatch({'operation': operation, 'runId': run, 'scope': 'a' * 64, **extra}, self.state)

    def complete(self, run='run_123'):
        until = time.monotonic() + 15
        while time.monotonic() < until:
            result = self.operation('query', run)
            if result['state'] not in ('starting', 'running', 'cancelling'):
                return result
            time.sleep(.05)
        self.fail('runner never completed')

    def tool_fixture(self, directory='external tools', label='original'):
        tools = self.root / directory
        tools.mkdir()
        compiler = tools / 'cc'
        compiler.write_text('#!/bin/sh\necho ' + label + '\nexec /usr/bin/cc "$@"\n')
        compiler.chmod(0o700)
        return tools, compiler

    def test_system_tool_records_real_bytes_and_absolute_path(self):
        tools, compiler = self.tool_fixture()
        with mock.patch.dict(os.environ, PATH=str(tools) + ':' + os.environ['PATH']):
            prepared = self.prepare()
        self.assertEqual(prepared['task']['executableSha256'], self.runner.digest(compiler.read_bytes()))
        self.assertEqual(prepared['task']['executablePath'], str(compiler.resolve()))

    def test_replaced_external_tool_rejects_old_confirmation_without_running(self):
        tools, compiler = self.tool_fixture()
        with mock.patch.dict(os.environ, PATH=str(tools) + ':' + os.environ['PATH']):
            prepared = self.prepare()
            before = (self.state / 'run_123/prepared.json').read_bytes()
            compiler.write_text(compiler.read_text() + '# upgraded\n')
            with self.assertRaisesRegex(ValueError, 'SOURCE_CHANGED'):
                self.operation('start', confirmation=prepared['confirmation'])
            self.assertEqual(self.operation('query')['state'], 'prepared')
            self.assertEqual(before, (self.state / 'run_123/prepared.json').read_bytes())
            self.assertFalse((self.repo / 'app').exists())

    def test_external_tool_change_is_readonly_stale_observation_after_completion(self):
        tools, compiler = self.tool_fixture()
        with mock.patch.dict(os.environ, PATH=str(tools) + ':' + os.environ['PATH']):
            prepared = self.prepare()
            self.operation('start', confirmation=prepared['confirmation'])
            done = self.complete()
            self.assertEqual(done['state'], 'succeeded', done)
            before = {p.name: p.read_bytes() for p in (self.state / 'run_123').iterdir() if p.is_file()}
            compiler.write_text(compiler.read_text() + '# upgraded\n')
            current = self.operation('inspect')
            self.assertNotEqual(current['executableSha256'], done['task']['executableSha256'])
            self.assertEqual(current['executionContext']['fingerprint'], done['executionContext']['fingerprint'])
            self.assertEqual(before, {p.name: p.read_bytes() for p in (self.state / 'run_123').iterdir() if p.is_file()})

    def test_worker_rechecks_tool_after_start_acceptance(self):
        tools, compiler = self.tool_fixture()
        with mock.patch.dict(os.environ, PATH=str(tools) + ':' + os.environ['PATH']):
            self.prepare()
            compiler.write_text(compiler.read_text() + '# replaced before worker\n')
            self.runner.worker(self.state / 'run_123')
            done = self.operation('query')
            self.assertEqual(done['state'], 'failed')
            self.assertEqual(done['reason'], 'SOURCE_CHANGED')
            self.assertFalse((self.repo / 'app').exists())

    def test_path_and_symlink_retarget_require_new_confirmation_even_for_same_bytes(self):
        tools, compiler = self.tool_fixture()
        other, alternate = self.tool_fixture('replacement tools')
        original_path = os.environ['PATH']
        with mock.patch.dict(os.environ, PATH=str(tools) + ':' + original_path):
            prepared = self.prepare()
            os.environ['PATH'] = str(other) + ':' + original_path
            with self.assertRaisesRegex(ValueError, 'SOURCE_CHANGED'):
                self.operation('start', confirmation=prepared['confirmation'])
            os.environ['PATH'] = str(tools) + ':' + original_path
            compiler.unlink()
            compiler.symlink_to(alternate)
            with self.assertRaisesRegex(ValueError, 'SOURCE_CHANGED'):
                self.operation('start', confirmation=prepared['confirmation'])
            self.assertFalse((self.repo / 'app').exists())

    def test_missing_and_nonexecutable_system_tool_fail_before_preparing(self):
        tools, compiler = self.tool_fixture()
        (tools / 'git').symlink_to('/usr/bin/git')
        for mode in (0o600, None):
            if mode is None:
                compiler.unlink()
            else:
                compiler.chmod(mode)
            with mock.patch.dict(os.environ, PATH=str(tools)):
                with self.assertRaisesRegex(ValueError, 'EXECUTABLE_UNAVAILABLE'):
                    self.prepare()
            self.assertFalse((self.state / 'run_123/prepared.json').exists())

    def test_execute_uses_observed_tool_even_if_path_changes_before_spawn(self):
        tools, compiler = self.tool_fixture()
        other, _ = self.tool_fixture('replacement tools', 'unexpected')
        with mock.patch.dict(os.environ, PATH=str(tools) + ':' + os.environ['PATH']):
            prepared = self.prepare()
            os.environ['PATH'] = str(other) + ':' + os.environ['PATH']
            job = self.state / 'run_123'
            with self.runner.lock(job / 'test.lock') as lock:
                self.runner.execute(job, prepared['task'], lock)
            done = self.operation('query')
            self.assertEqual(done['state'], 'succeeded', done)
            self.assertIn('original', done['log'])
            self.assertNotIn('unexpected', done['log'])

    def test_relative_path_entries_are_resolved_from_task_working_directory(self):
        tools, compiler = self.tool_fixture()
        with mock.patch.dict(os.environ, PATH='../external tools'):
            observed = self.runner.system_executable('cc', self.repo)
        self.assertEqual(observed['executablePath'], str(compiler))
        self.assertEqual(observed['executableSha256'], self.runner.digest(compiler.read_bytes()))

    def test_tool_observation_rejects_special_large_and_mid_read_changed_files(self):
        tools, compiler = self.tool_fixture()
        with mock.patch.dict(os.environ, PATH=str(tools)):
            with compiler.open('ab') as stream:
                stream.write(b'#' * 600)
            with mock.patch.object(self.runner, 'LIMIT', 1):
                with self.assertRaisesRegex(ValueError, 'EXECUTABLE_SIZE_LIMIT'):
                    self.runner.system_executable('cc', self.repo)
            with mock.patch.object(self.runner.time, 'monotonic', side_effect=[0, 16]):
                with self.assertRaisesRegex(ValueError, 'EXECUTABLE_READ_LIMIT'):
                    self.runner.system_executable('cc', self.repo)
            original_hash = self.runner.hashlib.sha256
            class ChangingHash:
                def __init__(self):
                    self.value = original_hash()
                def update(self, part):
                    self.value.update(part)
                    compiler.chmod(0o755)
                def hexdigest(self):
                    return self.value.hexdigest()
            with mock.patch.object(self.runner.hashlib, 'sha256', ChangingHash):
                with self.assertRaisesRegex(ValueError, 'EXECUTABLE_CHANGED'):
                    self.runner.system_executable('cc', self.repo)
            compiler.unlink()
            os.mkfifo(compiler, 0o700)
            with self.assertRaisesRegex(ValueError, 'EXECUTABLE_INVALID'):
                self.runner.system_executable('cc', self.repo)

    def test_log_redaction_precedes_tail_and_hash_covers_original(self):
        self.prepare()
        job = self.state / 'run_123'
        raw = ('-----BEGIN PRIVATE KEY-----\n' + 'canary-key\n' * 4000 +
               '-----END PRIVATE KEY-----\n' + 'x' * 40000 +
               '\npassword=canary-password\nfinal compiler error').encode()
        (job / 'output.log').write_bytes(raw)
        result = self.operation('query')
        self.assertNotIn('canary', result['log'])
        self.assertTrue(result['log'].endswith('final compiler error'))
        self.assertLessEqual(len(result['log'].encode()), 32768)
        self.assertEqual(result['logBytes'], len(raw))
        self.assertEqual(result['logSha256'], self.runner.digest(raw))
        raw = ('-----BEGIN PRIVATE KEY-----\n' + 'canary-key\n' * 4000).encode()
        (job / 'output.log').write_bytes(raw)
        self.assertNotIn('canary', self.operation('query')['log'])
        raw = ('诊断' * 10000 + '\nfinal diagnostic').encode()
        (job / 'output.log').write_bytes(raw)
        result = self.operation('query')
        self.assertLessEqual(len(result['log'].encode()), 32768)
        self.assertTrue(result['log'].endswith('final diagnostic'))
        (job / 'output.log').write_bytes(b'x' * (self.runner.LIMIT + 1))
        with self.assertRaisesRegex(ValueError, 'LOG_SIZE_LIMIT'):
            self.operation('query')

    def test_build_persists_and_duplicate_start_does_not_rerun(self):
        prepared = self.prepare()
        self.operation('start', confirmation=prepared['confirmation'])
        done = self.complete()
        self.assertEqual(done['state'], 'succeeded', done)
        self.assertEqual(done['exitCode'], 0)
        self.assertEqual(len(done['artifacts'][0]['sha256']), 64)
        stamp = (self.repo / 'app').stat().st_mtime_ns
        self.operation('start', confirmation=prepared['confirmation'])
        self.assertEqual((self.repo / 'app').stat().st_mtime_ns, stamp)

    def test_inspect_completed_task_is_readonly_and_detects_source_drift(self):
        p = self.prepare()
        self.operation('start', confirmation=p['confirmation'])
        done = self.complete()
        job = self.state / 'run_123'
        before = {p.name:p.read_bytes() for p in job.iterdir() if p.is_file()}
        stamp = (self.repo / 'app').stat().st_mtime_ns
        inspected = self.operation('inspect')
        self.assertEqual(set(inspected), {'runId','scope','executionContext','executableSha256'})
        self.assertEqual(inspected['executionContext']['fingerprint'], done['executionContext']['fingerprint'])
        self.assertEqual(inspected['executableSha256'], done['task']['executableSha256'])
        (self.repo / 'main.c').write_text('int main(){return 2;}\n')
        changed = self.operation('inspect')
        self.assertNotEqual(changed['executionContext']['fingerprint'], inspected['executionContext']['fingerprint'])
        self.assertEqual((self.repo / 'app').stat().st_mtime_ns, stamp)
        self.assertEqual(before, {p.name:p.read_bytes() for p in job.iterdir() if p.is_file()})
        self.assertNotIn(str(self.repo), json.dumps(changed))

    def test_inspect_detects_executable_change_without_treating_build_output_as_source(self):
        p = self.prepare()
        self.operation('start', confirmation=p['confirmation'])
        self.complete()
        p = self.prepare('run', 'run_binary')
        self.operation('start', 'run_binary', confirmation=p['confirmation'])
        done = self.complete('run_binary')
        with (self.repo / 'app').open('ab') as stream:
            stream.write(b'modified')
        inspected = self.operation('inspect', 'run_binary')
        self.assertEqual(inspected['executionContext']['fingerprint'], done['executionContext']['fingerprint'])
        self.assertNotEqual(inspected['executableSha256'], done['task']['executableSha256'])

    def test_inspect_rejects_unfinished_wrong_scope_and_changed_definition(self):
        p = self.prepare()
        with self.assertRaisesRegex(ValueError, 'RUN_NOT_FINISHED'):
            self.operation('inspect')
        with self.assertRaisesRegex(ValueError, 'SCOPE_MISMATCH'):
            self.runner.dispatch({'operation':'inspect','runId':'run_123','scope':'b'*64},self.state)
        self.operation('start', confirmation=p['confirmation'])
        self.complete()
        (self.repo / '.lynx/tasks.json').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'TASK_DEFINITION_CHANGED'):
            self.operation('inspect')

    def test_build_and_run_share_actual_context_without_generated_output_drift(self):
        # Unignored generated files must not split one unchanged source revision.
        (self.repo / '.gitignore').write_text('')
        self.git('add', '.gitignore')
        self.git('commit', '-qm', 'track generated outputs through reviewed tasks')
        self.commit = self.git('rev-parse', 'HEAD').strip()
        build = self.prepare('build', 'build_context')
        before = time.time()
        self.operation('start', 'build_context', confirmation=build['confirmation'])
        built = self.complete('build_context')
        run = self.prepare('run', 'run_context')
        self.operation('start', 'run_context', confirmation=run['confirmation'])
        done = self.complete('run_context')
        self.assertEqual(built['state'], 'succeeded', built)
        self.assertEqual(done['state'], 'succeeded', done)
        self.assertEqual(built['executionContext']['fingerprint'], done['executionContext']['fingerprint'])
        self.assertNotEqual(built['task']['sourceFingerprint'], done['task']['sourceFingerprint'])
        self.assertEqual(built['executionContext']['sourceCommit'], self.commit)
        self.assertEqual(built['executionContext']['sourceFingerprint'], built['task']['learningSourceFingerprint'])
        self.assertEqual(built['executionContext']['facts']['architecture'], os.uname().machine)
        observed = datetime.fromisoformat(built['executionContext']['observedAt'].replace('Z', '+00:00')).timestamp()
        started = datetime.fromisoformat(built['startedAt'].replace('Z', '+00:00')).timestamp()
        self.assertLessEqual(before, observed)
        self.assertLessEqual(observed, started)
        facts = {k: v for k, v in built['executionContext'].items() if k not in {'observedAt', 'fingerprint'}}
        self.assertEqual(self.runner.digest(json.dumps(facts, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()), built['executionContext']['fingerprint'])
        self.assertEqual(self.operation('query', 'build_context')['executionContext'], built['executionContext'])
        self.assertNotIn(str(self.repo), json.dumps(built['executionContext']))
        self.assertNotIn('workspace', built['executionContext'])

    def test_common_input_identity_keeps_tracked_and_undeclared_source(self):
        tasks = json.loads((self.repo / '.lynx/tasks.json').read_text())
        tasks['tasks'].append({'id': 'source_output', 'argv': ['make'], 'timeoutSeconds': 1, 'outputs': ['main.c']})
        (self.repo / '.lynx/tasks.json').write_text(json.dumps(tasks))
        self.git('add', '.lynx/tasks.json')
        self.git('commit', '-qm', 'declared output cannot hide tracked source')
        self.commit = self.git('rev-parse', 'HEAD').strip()
        original = self.prepare('build', 'source_before')['task']['learningSourceFingerprint']
        (self.repo / 'main.c').write_text('int main(){return 1;}\n')
        changed = self.prepare('build', 'source_after')['task']['learningSourceFingerprint']
        self.assertNotEqual(original, changed)
        (self.repo / 'new_input.h').write_text('#define INPUT 1\n')
        self.assertNotEqual(changed, self.prepare('build', 'untracked_source')['task']['learningSourceFingerprint'])

    def test_environment_drift_prevents_start_and_old_records_get_no_context(self):
        from unittest.mock import patch
        prepared = self.prepare()
        facts = {**prepared['task']['environmentFacts'], 'architecture': 'changed-architecture'}
        with patch.object(self.runner, 'environment_facts', return_value=facts):
            with self.assertRaisesRegex(ValueError, 'SOURCE_CHANGED'):
                self.operation('start', confirmation=prepared['confirmation'])
        self.assertEqual(self.operation('query')['state'], 'prepared')
        self.operation('start', confirmation=prepared['confirmation'])
        done = self.complete()
        self.assertIn('executionContext', done)
        job = self.state / 'run_123'
        status = json.loads((job / 'status.json').read_text())
        status.pop('executionContext')
        (job / 'status.json').write_text(json.dumps(status))
        self.assertNotIn('executionContext', self.operation('query'))

    def test_process_times_and_separate_stdout_evidence_survive_reopen(self):
        source = r'''#include <stdio.h>
int main(){fputs("START-STDOUT\n",stdout);for(int i=0;i<70000;i++)putchar('x');fputs("\npassword=canary-secret\nEND-STDOUT\n",stdout);fputs("STDERR-ONLY\n",stderr);return 0;}
'''
        (self.repo / 'probe.c').write_text(source)
        subprocess.run(['cc', str(self.repo / 'probe.c'), '-o', str(self.repo / 'app')], check=True)
        before = time.time()
        p = self.prepare('run')
        self.operation('start', confirmation=p['confirmation'])
        done = self.complete()
        self.assertEqual(done['state'], 'succeeded', done)
        started = datetime.fromisoformat(done['startedAt'].replace('Z', '+00:00')).timestamp()
        finished = datetime.fromisoformat(done['finishedAt'].replace('Z', '+00:00')).timestamp()
        self.assertLessEqual(before, started)
        self.assertLessEqual(started, finished)
        self.assertLessEqual(finished, time.time())
        self.assertEqual(self.operation('query')['startedAt'], done['startedAt'])
        self.assertEqual(self.operation('query')['finishedAt'], done['finishedAt'])
        payload = b''
        expected = ''
        expected_raw = ''
        while True:
            part = self.operation('evidence', kind='stdout', offset=len(payload), length=16384, expectedSha256=expected, expectedRawSha256=expected_raw)
            if expected:
                self.assertEqual(part['sha256'], expected)
            expected = part['sha256']
            expected_raw = part['rawSha256']
            self.assertLess(len(json.dumps(part).encode()), 131072)
            payload += base64.b64decode(part['dataBase64'], validate=True)
            if part['nextOffset'] == part['size']:
                break
        self.assertEqual(payload, b'START-STDOUT\n' + b'x' * 70000 + b'\n[REDACTED CREDENTIAL LINE]\nEND-STDOUT\n')
        self.assertEqual(self.runner.digest(payload), expected)
        self.assertTrue(part['redacted'])
        self.assertFalse(part['outputTruncated'])
        self.assertNotEqual(part['rawSha256'], expected)
        self.assertEqual(part['rawBytes'], len((self.state / 'run_123/stdout.log').read_bytes()))
        combined = self.operation('evidence', kind='build_log', offset=0, length=49152)
        self.assertIn(b'STDERR-ONLY', (self.state / 'run_123/output.log').read_bytes())
        self.assertEqual(combined['contentType'], 'text/plain')
        exit_status = self.operation('evidence', kind='exit_status', offset=0, length=10)
        self.assertEqual(base64.b64decode(exit_status['dataBase64']), b'0')
        self.assertEqual(exit_status['contentType'], 'application/json')

    def test_evidence_requires_terminal_process_and_original_scope(self):
        p = self.prepare('long')
        with self.assertRaisesRegex(ValueError, 'EVIDENCE_NOT_AVAILABLE'):
            self.operation('evidence', kind='stdout', offset=0, length=10)
        self.operation('start', confirmation=p['confirmation'])
        until = time.monotonic() + 5
        while self.operation('query')['state'] == 'starting' and time.monotonic() < until:
            time.sleep(.02)
        with self.assertRaisesRegex(ValueError, 'EVIDENCE_NOT_AVAILABLE'):
            self.operation('evidence', kind='stdout', offset=0, length=10)
        self.operation('cancel')
        self.assertEqual(self.complete()['state'], 'cancelled')
        with self.assertRaisesRegex(ValueError, 'SCOPE_MISMATCH'):
            self.runner.dispatch({'operation':'evidence','runId':'run_123','scope':'b'*64,'kind':'stdout','offset':0,'length':10},self.state)
        self.assertIsInstance(json.loads(base64.b64decode(self.operation('evidence',kind='exit_status',offset=0,length=20)['dataBase64'])), int)
        self.prepare('build','never_started')
        self.operation('cancel','never_started')
        with self.assertRaisesRegex(ValueError, 'EVIDENCE_NOT_AVAILABLE'):
            self.operation('evidence','never_started',kind='exit_status',offset=0,length=20)

    def test_dual_stream_output_limit_bounds_both_files_and_marks_truncation(self):
        source = '#include <stdio.h>\nint main(){for(int i=0;i<3000000;i++){fputc(83,stdout);fputc(69,stderr);}return 0;}\n'
        (self.repo/'streams.c').write_text(source)
        subprocess.run(['cc',str(self.repo/'streams.c'),'-o',str(self.repo/'app')],check=True)
        p = self.prepare('run')
        self.operation('start',confirmation=p['confirmation'])
        done = self.complete()
        self.assertEqual(done['state'],'output_limit',done)
        for name in ['output.log','stdout.log']:
            self.assertLessEqual((self.state/'run_123'/name).stat().st_size,self.runner.LIMIT)
        stdout = (self.state/'run_123/stdout.log').read_bytes()
        self.assertTrue(stdout)
        self.assertEqual(set(stdout),{83})
        part = self.operation('evidence',kind='stdout',offset=0,length=49152)
        self.assertTrue(part['outputTruncated'])
        self.assertEqual(part['state'],'output_limit')

    def test_stderr_is_drained_after_stdout_closes(self):
        source = '#include <stdio.h>\n#include <unistd.h>\nint main(){close(1);for(int i=0;i<80000;i++)fputc(69,stderr);fputs("END-ERROR",stderr);return 0;}\n'
        (self.repo/'stderr.c').write_text(source)
        subprocess.run(['cc',str(self.repo/'stderr.c'),'-o',str(self.repo/'app')],check=True)
        p = self.prepare('run')
        self.operation('start',confirmation=p['confirmation'])
        done = self.complete()
        self.assertEqual(done['state'],'succeeded',done)
        self.assertTrue(done['log'].endswith('END-ERROR'))
        self.assertEqual((self.state/'run_123/stdout.log').read_bytes(),b'')
        part = self.operation('evidence',kind='stdout',offset=0,length=10)
        self.assertEqual(part['size'],0)
        self.assertEqual(part['dataBase64'],'')

    def test_chunk_digest_bounds_and_redaction_are_checked_before_slicing(self):
        p = self.prepare()
        self.operation('start', confirmation=p['confirmation'])
        self.assertEqual(self.complete()['state'], 'succeeded')
        file = self.state / 'run_123/stdout.log'
        file.write_bytes(b'prefix\n-----BEGIN PRIVATE KEY-----\n' + b'canary\n' * 6000 + b'-----END PRIVATE KEY-----\nfinish\n')
        part = self.operation('evidence',kind='stdout',offset=0,length=49152)
        content = base64.b64decode(part['dataBase64'])
        self.assertEqual(content,b'prefix\n[REDACTED PRIVATE KEY]\nfinish\n')
        original = part
        file.write_bytes(file.read_bytes().replace(b'canary',b'mutant'))
        same_clean = self.operation('evidence',kind='stdout',offset=0,length=49152)
        self.assertEqual(same_clean['sha256'],original['sha256'])
        self.assertNotEqual(same_clean['rawSha256'],original['rawSha256'])
        with self.assertRaisesRegex(ValueError,'EVIDENCE_CHANGED'):
            self.operation('evidence',kind='stdout',offset=1,length=10,expectedSha256=original['sha256'],expectedRawSha256=original['rawSha256'])
        with self.assertRaisesRegex(ValueError,'EVIDENCE_CHANGED'):
            self.operation('evidence',kind='stdout',offset=1,length=10)
        file.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'EVIDENCE_CHANGED'):
            self.operation('evidence',kind='stdout',offset=0,length=10,expectedSha256=part['sha256'])
        for extra in [{'kind':'../output.log'},{'offset':-1},{'offset':1000000},{'length':0},{'length':49153},{'offset':True}]:
            request = {'kind':'stdout','offset':0,'length':10,**extra}
            with self.assertRaises(ValueError):
                self.operation('evidence',**request)
        file.write_bytes(b'x'*(self.runner.LIMIT+1))
        with self.assertRaisesRegex(ValueError,'LOG_SIZE_LIMIT'):
            self.operation('evidence',kind='stdout',offset=0,length=10)
        file.unlink()
        file.symlink_to(self.repo/'main.c')
        with self.assertRaisesRegex(ValueError,'EVIDENCE_NOT_AVAILABLE'):
            self.operation('evidence',kind='stdout',offset=0,length=10)
        file.unlink()
        os.mkfifo(file)
        with self.assertRaisesRegex(ValueError,'EVIDENCE_NOT_AVAILABLE'):
            self.operation('evidence',kind='stdout',offset=0,length=10)

    def test_changed_untracked_source_invalidates_confirmation(self):
        (self.repo / 'extra.c').write_text('old')
        prepared = self.prepare()
        (self.repo / 'extra.c').write_text('new')
        with self.assertRaisesRegex(ValueError, 'SOURCE_CHANGED'):
            self.operation('start', confirmation=prepared['confirmation'])
        self.assertFalse((self.repo / 'app').exists())

    def test_task_definition_must_match_reviewed_commit(self):
        path = self.repo / '.lynx/tasks.json'
        path.write_text(path.read_text().replace('main.c', 'evil.c'))
        with self.assertRaisesRegex(ValueError, 'TASK_DEFINITION_CHANGED'):
            self.prepare()

    def test_timeout_kills_process_group_and_records_failure(self):
        p = self.prepare('wait')
        self.operation('start', confirmation=p['confirmation'])
        done = self.complete()
        self.assertEqual(done['state'], 'timed_out', done)

    def test_cancel_and_project_lock(self):
        p = self.prepare('long')
        self.operation('start', confirmation=p['confirmation'])
        until = time.monotonic() + 5
        while self.operation('query')['state'] == 'starting' and time.monotonic() < until:
            time.sleep(.02)
        other = self.prepare('build', 'run_other')
        self.operation('start', 'run_other', confirmation=other['confirmation'])
        self.assertEqual(self.complete('run_other')['state'], 'busy')
        self.operation('cancel')
        self.assertEqual(self.complete()['state'], 'cancelled')

    def test_wrong_scope_cannot_query_or_cancel(self):
        self.prepare()
        for operation in ('query', 'cancel', 'start'):
            with self.assertRaisesRegex(ValueError, 'SCOPE_MISMATCH'):
                self.runner.dispatch({'operation': operation, 'runId': 'run_123', 'scope': 'b' * 64}, self.state)

    def test_existing_workbench_and_learn_share_workspace_lock(self):
        p = self.prepare('long')
        self.operation('start', confirmation=p['confirmation'])
        until = time.monotonic() + 5
        while self.operation('query')['state'] == 'starting' and time.monotonic() < until:
            time.sleep(.02)
        wrapper = SOURCE.parents[2] / 'workbench/project_lock.py'
        result = subprocess.run(['python3', str(wrapper), str(self.repo), 'touch should-not-exist'], capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b'LYNX_PROJECT_BUSY', result.stderr)
        self.assertFalse((self.repo / 'should-not-exist').exists())
        self.operation('cancel')
        self.assertEqual(self.complete()['state'], 'cancelled')

    def test_output_limit_stops_task(self):
        p = self.prepare('flood')
        self.operation('start', confirmation=p['confirmation'])
        done = self.complete()
        self.assertEqual(done['state'], 'output_limit')
        self.assertLessEqual((self.state / 'run_123/output.log').stat().st_size, 1048576)

    def test_symlink_output_escape_is_rejected(self):
        (self.repo / 'app').symlink_to(self.root / 'outside')
        with self.assertRaisesRegex(ValueError, 'PATH_ESCAPE'):
            self.prepare()

    def test_lost_worker_is_unknown_and_start_never_replays_it(self):
        p = self.prepare()
        self.runner.atomic(self.state / 'run_123/status.json', {'state': 'starting', 'updatedAt': 0})
        self.assertEqual(self.operation('query')['state'], 'unknown')
        self.assertEqual(self.operation('start', confirmation=p['confirmation'])['state'], 'unknown')
        self.assertFalse((self.repo / 'app').exists())

    def test_run_binary_change_invalidates_prepared_task(self):
        p = self.prepare()
        self.operation('start', confirmation=p['confirmation'])
        self.assertEqual(self.complete()['state'], 'succeeded')
        run = self.prepare('run', 'run_binary')
        with (self.repo / 'app').open('ab') as output:
            output.write(b'changed')
        with self.assertRaisesRegex(ValueError, 'SOURCE_CHANGED'):
            self.operation('start', 'run_binary', confirmation=run['confirmation'])

    def test_cancel_after_process_closes_output_still_stops_promptly(self):
        # A real executable closes its output, then keeps working. EOF is not exit.
        source = '#include <unistd.h>\n#include <stdio.h>\nint main(){close(1);close(2);fclose(fopen("output.closed","w"));sleep(8);return 0;}\n'
        (self.repo / 'silent.c').write_text(source)
        subprocess.run(['cc', str(self.repo / 'silent.c'), '-o', str(self.repo / 'app')], check=True)
        prepared = self.prepare('run')
        self.operation('start', confirmation=prepared['confirmation'])
        until = time.monotonic() + 3
        while self.operation('query')['state'] == 'starting' and time.monotonic() < until:
            time.sleep(.02)
        while not (self.repo / 'output.closed').exists() and time.monotonic() < until:
            time.sleep(.02)
        self.assertTrue((self.repo / 'output.closed').exists(), 'child never closed output')
        start = time.monotonic()
        self.operation('cancel')
        done = self.complete()
        self.assertEqual(done['state'], 'cancelled', done)
        self.assertLess(time.monotonic() - start, 2, 'cancellation waited for task timeout')


if __name__ == '__main__':
    unittest.main()
