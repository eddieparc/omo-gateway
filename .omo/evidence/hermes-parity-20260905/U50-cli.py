import os
from pathlib import Path
import subprocess
import tempfile

binary = Path('target/debug/omo-gateway').resolve()
with tempfile.TemporaryDirectory(prefix='u50-cli-') as temporary:
    root = Path(temporary)
    home = root / 'home'
    home.mkdir()
    hermes = home / '.hermes'
    hermes.mkdir()
    (hermes / 'config.yaml').write_text('model:\n  default: fixture-new\n  api_key: private-fixture\n')
    target = root / '.env'
    original = b'DEFAULT_MODEL=fixture-old\n'
    target.write_bytes(original)
    target.chmod(0o600)
    old = root / 'old-inode'
    os.link(target, old)
    environment = {'PATH': os.environ['PATH'], 'HOME': str(home), 'HERMES_HOME': str(hermes), 'DATABASE_URL': 'sqlite://' + str(root / 'fixture.db')}
    for index in range(2):
        result = subprocess.run([str(binary), 'migrate', '--no-cutover'], cwd=root, env=environment, capture_output=True, text=True, timeout=60)
        print(f'CLI import {index + 1} exit={result.returncode}')
        print(result.stdout)
        print(result.stderr)
        assert result.returncode == 0
    backups = list(root.glob('.env.bak-*'))
    assert len(backups) == 2
    assert original in [p.read_bytes() for p in backups]
    assert old.read_bytes() == original
    for path in [target, *backups]:
        mode = path.stat().st_mode & 0o777
        print(f'{path.name} mode={mode:o}')
        assert mode == 0o600
    assert not list(root.glob('*.tmp-omon-migration-*'))
print('CLI fixture removed; no real HOME/state/services touched')
