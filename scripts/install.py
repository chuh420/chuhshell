#!/usr/bin/env python3
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
HOME_DIR = Path.home()
CONFIG = Path(os.environ.get('XDG_CONFIG_HOME', HOME_DIR / '.config'))
DATA = Path(os.environ.get('XDG_DATA_HOME', HOME_DIR / '.local/share'))
STATE = Path(os.environ.get('XDG_STATE_HOME', HOME_DIR / '.local/state')) / 'chuhshell/installation'
MANIFEST = STATE / 'manifest.json'


def run(*args, check=True):
    return subprocess.run(args, check=check, text=True, capture_output=True)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def write(path, data, mode):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + '.chuhshell-new')
    temporary.write_bytes(data)
    temporary.chmod(mode)
    temporary.replace(path)


def save(path, data, mode, manifest):
    key = str(path)
    if key not in manifest:
        backup = STATE / digest(key.encode())
        exists = path.exists()
        if exists:
            shutil.copy2(path, backup)
        manifest[key] = {'existed': exists, 'backup': str(backup), 'mode': path.stat().st_mode & 0o777 if exists else mode}
    write(path, data, mode)
    manifest[key]['installed'] = digest(data)


def restore(manifest, respect_edits):
    remaining = {}
    for filename, entry in manifest.items():
        path = Path(filename)
        if respect_edits and path.exists() and digest(path.read_bytes()) != entry['installed']:
            print(f'Preserved modified file: {path}')
            remaining[filename] = entry
            continue
        if entry['existed']:
            write(path, Path(entry['backup']).read_bytes(), entry['mode'])
        else:
            path.unlink(missing_ok=True)
    return remaining


def stop_old_shell():
    reply = run('busctl', '--user', 'call', 'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'GetConnectionUnixProcessID', 's', 'dev.chuh.chuhshell', check=False)
    if reply.returncode:
        return
    pid = int(reply.stdout.split()[-1])
    helpers = set()
    for task in (Path('/proc') / str(pid) / 'task').glob('*'):
        try:
            helpers.update((task / 'children').read_text().split())
        except OSError:
            pass
    for helper in helpers:
        try:
            command = (Path('/proc') / helper / 'cmdline').read_bytes().split(b'\0')
            if command and ((Path(os.fsdecode(command[0])).name == 'pactl' and b'subscribe' in command) or (Path(os.fsdecode(command[0])).name == 'udevadm' and b'monitor' in command)):
                os.kill(int(helper), signal.SIGTERM)
        except (OSError, ValueError):
            pass
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    for _ in range(50):
        if not Path(f'/proc/{pid}').exists():
            return
        time.sleep(0.1)
    raise RuntimeError('The previous shell has not stopped')


def menu_bindings(text, binary):
    text = re.sub(r'^[ \t]*Mod\+Shift\+D\b[^\n{]*\{[^{}]*\}[ \t]*;?[ \t]*\n?', '', text, flags=re.MULTILINE)
    binding = '    Mod+Space hotkey-overlay-title="chuh menu" { spawn ' + json.dumps(str(binary)) + ' "menu"; }'
    text, count = re.subn(r'^[ \t]*Mod\+Space\b[^\n{]*\{[^{}]*\}[ \t]*;?[ \t]*$', lambda _: binding, text, flags=re.MULTILINE)
    if not count:
        text, count = re.subn(r'(^[ \t]*binds[ \t]*\{)', lambda match: match[1] + '\n' + binding, text, count=1, flags=re.MULTILINE)
        if not count:
            raise RuntimeError('Could not find the Niri binds block')
    return text


def install():
    binary = ROOT / 'target/release/chuhshell'
    if not binary.exists():
        raise RuntimeError('Build first: cargo build --release --locked')
    STATE.mkdir(parents=True, exist_ok=True, mode=0o700)
    manifest = json.loads(MANIFEST.read_text()) if MANIFEST.exists() else {}
    destination = HOME_DIR / '.local/bin/chuhshell'
    service_path = CONFIG / 'systemd/user/chuhshell.service'
    notification_path = DATA / 'dbus-1/services/org.freedesktop.Notifications.service'
    niri_path = (CONFIG / 'niri/config.kdl').resolve()
    service = (ROOT / 'packaging/chuhshell.service').read_text().replace('ExecStart=/usr/bin/chuhshell', 'ExecStart=' + json.dumps(str(destination)))
    notification = (ROOT / 'packaging/org.freedesktop.Notifications.service').read_text().replace('Exec=/usr/bin/chuhshell', 'Exec=' + json.dumps(str(destination)))
    changes = [(destination, binary.read_bytes(), 0o755), (service_path, service.encode(), 0o644), (notification_path, notification.encode(), 0o644)]
    if niri_path.exists():
        text = menu_bindings(niri_path.read_text(), destination)
        replacement = 'spawn-at-startup "systemctl" "--user" "start" "chuhshell.service"'
        text, count = re.subn(r'^\s*spawn-at-startup\s+"[^"\n]*chuhshell"\s*;?\s*$', replacement, text, flags=re.MULTILINE)
        if not count and replacement not in text:
            text += '\n' + replacement + '\n'
        changes.append((niri_path, text.encode(), niri_path.stat().st_mode & 0o777))
    before = {path: (path.read_bytes(), path.stat().st_mode & 0o777) if path.exists() else None for path, _, _ in changes}
    enabled_before = run('systemctl', '--user', 'is-enabled', 'chuhshell.service', check=False).returncode == 0
    active_before = run('systemctl', '--user', 'is-active', 'chuhshell.service', check=False).returncode == 0
    stopped = False
    try:
        for path, data, mode in changes:
            save(path, data, mode, manifest)
        if niri_path.exists():
            run('niri', 'validate', '--config', str(niri_path))
        run('systemctl', '--user', 'daemon-reload')
        environment = [name for name in ['WAYLAND_DISPLAY', 'NIRI_SOCKET', 'DISPLAY', 'XDG_CURRENT_DESKTOP'] if name in os.environ]
        if environment:
            run('systemctl', '--user', 'import-environment', *environment)
        run('systemctl', '--user', 'stop', 'chuhshell.service', check=False)
        stopped = True
        stop_old_shell()
        run('systemctl', '--user', 'enable', 'chuhshell.service')
        run('systemctl', '--user', 'reset-failed', 'chuhshell.service', check=False)
        run('systemctl', '--user', 'start', 'chuhshell.service')
        time.sleep(1)
        run('systemctl', '--user', 'is-active', 'chuhshell.service')
        if 'NIRI_SOCKET' in os.environ:
            run('niri', 'msg', 'action', 'load-config-file')
        write(MANIFEST, json.dumps(manifest, indent=2).encode(), 0o600)
        print('Installed chuhshell and started its user service. Backups: ' + str(STATE))
    except Exception:
        if stopped:
            run('systemctl', '--user', 'stop', 'chuhshell.service', check=False)
        if stopped and not enabled_before:
            run('systemctl', '--user', 'disable', 'chuhshell.service', check=False)
        for path, previous in before.items():
            if previous:
                write(path, *previous)
            else:
                path.unlink(missing_ok=True)
        run('systemctl', '--user', 'daemon-reload', check=False)
        if stopped and active_before:
            run('systemctl', '--user', 'start', 'chuhshell.service', check=False)
        elif stopped and destination.exists() and 'NIRI_SOCKET' in os.environ:
            run('niri', 'msg', 'action', 'spawn', '--', str(destination), check=False)
        raise


def uninstall():
    if not MANIFEST.exists():
        raise RuntimeError('No installation manifest found')
    run('systemctl', '--user', 'disable', '--now', 'chuhshell.service', check=False)
    remaining = restore(json.loads(MANIFEST.read_text()), True)
    if remaining:
        write(MANIFEST, json.dumps(remaining, indent=2).encode(), 0o600)
    else:
        MANIFEST.unlink()
    run('systemctl', '--user', 'daemon-reload')
    if 'NIRI_SOCKET' in os.environ:
        run('niri', 'msg', 'action', 'load-config-file', check=False)
    print('Restored installation backups; independently modified files were preserved')


if __name__ == '__main__':
    try:
        if sys.argv[1:] == ['install']:
            install()
        elif sys.argv[1:] == ['uninstall']:
            uninstall()
        else:
            raise RuntimeError('Usage: install.py install|uninstall')
    except (RuntimeError, OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f'chuhshell installation failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError):
            print(error.stderr, file=sys.stderr)
        sys.exit(1)
