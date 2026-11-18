"""Exercise the actual Tauri updater against staged, signed installers without installing them."""
import argparse
import contextlib
import functools
import hashlib
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from urllib.parse import unquote, urlparse


def run(probe, stage):
    source = json.loads((stage / 'latest.json').read_text())
    results = []
    with tempfile.TemporaryDirectory(prefix='imager-ota-') as temp:
        root = Path(temp)
        for platform in source['platforms'].values():
            name = unquote(urlparse(platform['url']).path.split('/')[-1])
            (root / name).symlink_to(stage / name)
        handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=temp)
        with http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler) as server:
            base = 'http://127.0.0.1:' + str(server.server_port)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            with contextlib.ExitStack() as stack:
                stack.callback(thread.join)
                stack.callback(server.shutdown)
                for target, platform in source['platforms'].items():
                    name = unquote(urlparse(platform['url']).path.split('/')[-1])
                    platform['url'] = base + '/' + name
                (root / 'stage.json').write_text(json.dumps(source))
                def check(installed, target, expect, feed, should_fail=False):
                    ret = subprocess.run([str(probe), installed, target, expect, base+'/'+feed],
                                         text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                         timeout=90, env={**os.environ, 'NO_PROXY':'localhost,127.0.0.1'})
                    if should_fail:
                        assert ret.returncode and ('signature' in ret.stdout.lower() or 'verify' in ret.stdout.lower()), ret.stdout
                    else:
                        assert ret.returncode == 0, ret.stdout
                    results.append(dict(installed=installed,target=target,feed=feed,rejected=should_fail))
                for target, platform in source['platforms'].items():
                    name = platform['url'].split('/')[-1]
                    with (stage / name).open('rb') as stream:
                        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
                    check('0.0.0', target, digest, 'stage.json')
                    check(source['version'], target, 'current', 'stage.json')
                    altered = json.loads(json.dumps(source))
                    (root / 'altered.bin').write_bytes(b'corrupted installer')
                    altered['platforms'][target]['url'] = base+'/altered.bin'
                    (root / 'invalid.json').write_text(json.dumps(altered))
                    check('0.0.0',target,digest,'invalid.json',True)
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ('probe','stage','report'):
        parser.add_argument(arg, type=Path)
    args = parser.parse_args()
    args.report.write_text(json.dumps(run(args.probe.resolve(),args.stage.resolve()),indent=2)+'\n')
