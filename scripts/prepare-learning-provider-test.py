#!/usr/bin/env python3
"""Copy default provider configuration only into an already isolated native test.

No keyring reads/writes: the real application resolves its existing credential.
"""
import json
from pathlib import Path
import sqlite3
import sys
from urllib.parse import urlsplit

source = Path(sys.argv[1]).resolve(strict=True)
root = Path(sys.argv[2]).resolve(strict=True)
if root.parent != Path('/tmp') or not root.name.startswith('lynx-learn-runtime-'):
    raise ValueError('Expected an isolated native test directory')
target = (root / 'data/com.dshanpi.lynx/supervisor.sqlite').resolve(strict=True)
if not target.is_relative_to(root) or target == source:
    raise ValueError('Invalid isolated destination')
columns = ['id', 'name', 'preset', 'base_url', 'model', 'timeout_seconds', 'max_tokens',
           'task_cost_limit', 'enabled', 'is_default', 'input_cost_per_million', 'output_cost_per_million']
with sqlite3.connect(source.as_uri() + '?mode=ro', uri=True) as original:
    row = original.execute('SELECT ' + ','.join(columns) + ' FROM ai_provider_config WHERE enabled=1 ORDER BY is_default DESC,id LIMIT 1').fetchone()
if not row:
    raise ValueError('No existing enabled provider')
endpoint = urlsplit(row[3])
if endpoint.scheme != 'https' or not endpoint.hostname or endpoint.username or endpoint.password or endpoint.query or endpoint.fragment:
    raise ValueError('Live test requires the existing HTTPS provider without inline credentials')
with sqlite3.connect(target, timeout=10) as isolated:
    if isolated.execute('SELECT count(*) FROM ai_provider_config').fetchone()[0] != 0:
        raise ValueError('Only an empty isolated provider table can be initialized')
    isolated.execute('INSERT INTO ai_provider_config (' + ','.join(columns) + ') VALUES (' + ','.join('?' for _ in row) + ')', row)
print(json.dumps({'id': row[0], 'name': row[1], 'model': row[4], 'endpointHost': endpoint.hostname, 'sourceConfigurationChanged': False, 'keyringChanged': False}))
