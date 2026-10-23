#!/usr/bin/env python3
"""Exercise generated installer additions without downloading or installing an app."""
import sys
sys.dont_write_bytecode = True

import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('prepare', ROOT / 'scripts/prepare-desktop-installers.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    for package in ('logana-gui', 'logana-full'):
        (root / f'{package}-installer.sh').write_text('    say "everything\'s installed!"\n')
        (root / f'{package}-installer.ps1').write_text('  Write-Information "everything\'s installed!"\n')
    module.prepare(root)
    before = (root / 'logana-gui-installer.sh').read_bytes()
    module.prepare(root)
    assert before == (root / 'logana-gui-installer.sh').read_bytes()
    home = root / 'home with spaces'
    (home / 'Desktop').mkdir(parents=True)
    binary_dir = home / 'bin'
    binary_dir.mkdir()
    (binary_dir / 'logana-gui').write_text('#!/bin/sh\nexit 0\n')
    (binary_dir / 'logana-gui').chmod(0o755)
    script = '''ensure() { "$@" || exit 1; }
ignore() { "$@" || :; }
say() { :; }
_install_dir="$HOME/bin"
''' + before.decode()
    helpers = root / 'helpers'
    helpers.mkdir()
    (helpers / 'xdg-user-dir').write_text('#!/bin/sh\nprintf "%s" "$HOME/Desktop"\n')
    (helpers / 'xdg-user-dir').chmod(0o755)
    env = dict(os.environ, HOME=str(home), XDG_DATA_HOME=str(home / 'data'), PATH=f"{helpers}:{os.environ['PATH']}")
    subprocess.run(['sh'], input=script, text=True, env=env, check=True)
    desktop = home / 'data/applications/logana.desktop'
    assert f'Exec="{binary_dir}/logana-gui" %F' in desktop.read_text()
    assert (home / 'Desktop/logana.desktop').read_bytes() == desktop.read_bytes()
    assert (home / 'data/icons/hicolor/256x256/apps/logana.png').read_bytes() == (ROOT / 'assets/logana.png').read_bytes()
    # Emulate macOS's uname and base64 switches on the Linux test host.
    mac_script = '''uname() { echo Darwin; }
base64() { command base64 -d; }
''' + script
    subprocess.run(['sh'], input=mac_script, text=True, env=env, check=True)
    app = home / 'Applications/Logana.app'
    assert (app / 'Contents/MacOS/logana-gui').read_bytes() == (binary_dir / 'logana-gui').read_bytes()
    assert (app / 'Contents/Resources/logana.icns').read_bytes() == (ROOT / 'assets/logana.icns').read_bytes()
    ET.parse(app / 'Contents/Info.plist')
    assert (home / 'Desktop/Logana.app').resolve() == app
for package in ('logana-gui', 'logana-full'):
    tree = ET.parse(ROOT / f'crates/{package}/wix/main.wxs')
    ns = {'w': 'http://schemas.microsoft.com/wix/2006/wi'}
    assert len(tree.findall('.//w:Shortcut', ns)) == 2
print('Desktop installer checks passed')
