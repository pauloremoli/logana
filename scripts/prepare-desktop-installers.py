#!/usr/bin/env python3
"""Add desktop integration to cargo-dist's GUI installers before uploading."""
import base64
from pathlib import Path
import sys
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def prepare(directory):
    shell = (ROOT / 'scripts/desktop/install.sh').read_text()
    for token, asset in [('PNG', 'logana.png'), ('ICNS', 'logana.icns')]:
        shell = shell.replace(f'@{token}@', base64.b64encode((ROOT / 'assets' / asset).read_bytes()).decode())
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
    shell = shell.replace('@VERSION@', version)
    powershell = (ROOT / 'scripts/desktop/install.ps1').read_text()
    for package in ('logana-gui', 'logana-full'):
        for suffix, marker, addition in [
            ('sh', '    say "everything\'s installed!"', shell),
            ('ps1', '  Write-Information "everything\'s installed!"', powershell),
        ]:
            path = directory / f'{package}-installer.{suffix}'
            source = path.read_text()
            if '# Desktop integration' in source or '# Use the embedded executable icon' in source:
                continue
            if source.count(marker) != 1:
                raise ValueError(f'{path}: cargo-dist success marker changed')
            path.write_text(source.replace(marker, addition + '\n' + marker))


if __name__ == '__main__':
    prepare(Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / 'target/distrib')
