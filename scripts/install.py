#!/usr/bin/env python3
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import argparse
import fcntl
from storage import write, snapshot, restore as restore_file, sync_directory

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


def save(path, data, mode, manifest):
    key = str(path)
    if key not in manifest:
        backup = STATE / digest(key.encode())
        exists = path.exists()
        if exists:
            write(backup, path.read_bytes(), 0o600)
        manifest[key] = {'existed': exists, 'backup': str(backup), 'mode': path.stat().st_mode & 0o777 if exists else mode}
        if path.is_symlink():
            manifest[key]['original'] = snapshot(path)
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
        if 'original' in entry:
            restore_file(path, entry['original'])
        elif entry['existed']:
            write(path, Path(entry['backup']).read_bytes(), entry['mode'])
        else:
            path.unlink(missing_ok=True)
    return remaining


def shell_pid():
    reply = run('busctl', '--user', 'call', 'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'GetConnectionUnixProcessID', 's', 'dev.chuh.chuhshell', check=False)
    if reply.returncode:
        return None
    try:
        pid = int(reply.stdout.split()[-1])
        return pid if pid > 1 else None
    except (ValueError, IndexError):
        return None


def stop_old_shell():
    pid = shell_pid()
    if pid is None:
        return
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


def journal_path():
    return STATE / 'transaction.json'


def recover():
    path = journal_path()
    if not path.exists():
        return
    journal = json.loads(path.read_text())
    for filename, entry in journal['files'].items():
        target = Path(filename)
        current = snapshot(target)
        if current != entry['before'] and (not target.is_file() or target.is_symlink() or digest(target.read_bytes()) != entry['installed']):
            raise RuntimeError(f'Interrupted installation: {target} changed afterwards. Preserve your edits and resolve {path} before retrying.')
    if journal['stopped']:
        run('systemctl', '--user', 'stop', 'chuhshell.service', check=False)
    for filename, entry in journal['files'].items():
        restore_file(Path(filename), entry['before'])
    restore_file(MANIFEST, journal['manifest'])
    run('systemctl', '--user', 'daemon-reload')
    if journal['stopped']:
        run('systemctl', '--user', 'enable' if journal['enabled'] else 'disable', 'chuhshell.service')
        if journal['active']:
            run('systemctl', '--user', 'start', 'chuhshell.service')
        elif journal.get('standalone') and 'NIRI_SOCKET' in os.environ:
            run('niri', 'msg', 'action', 'spawn', '--', journal['standalone'])
    path.unlink()
    sync_directory(STATE)
    print('Recovered interrupted installation')


def plan_configuration(binary, destination, config, preserve):
    args = [str(binary), 'installation-plan', str(destination)]
    if config is not None:
        args += ['--config', str(config)]
    if preserve:
        args.append('--preserve')
    return json.loads(run(*args).stdout)


def install(config=None):
    binary = ROOT / 'target/release/chuhshell'
    if not binary.exists():
        raise RuntimeError('Build first: cargo build --release --locked')
    STATE.mkdir(parents=True, exist_ok=True, mode=0o700)
    recover()
    manifest = json.loads(MANIFEST.read_text()) if MANIFEST.exists() else {}
    destination = HOME_DIR / '.local/bin/chuhshell'
    service_path = CONFIG / 'systemd/user/chuhshell.service'
    notification_path = DATA / 'dbus-1/services/org.freedesktop.Notifications.service'
    service = (ROOT / 'packaging/chuhshell.service').read_text().replace('ExecStart=/usr/bin/chuhshell', 'ExecStart=' + json.dumps(str(destination)))
    notification = (ROOT / 'packaging/org.freedesktop.Notifications.service').read_text().replace('Exec=/usr/bin/chuhshell', 'Exec=' + json.dumps(str(destination)))
    changes = [(destination, binary.read_bytes(), 0o755), (service_path, service.encode(), 0o644), (notification_path, notification.encode(), 0o644)]
    plan = plan_configuration(binary, destination, config, bool(manifest))
    for entry in plan:
        path = Path(entry['path'])
        if path.read_text() != entry['before']:
            raise RuntimeError(f'Niri configuration changed while preparing installation: {path}')
        if entry['text'] != entry['before']:
            changes.append((path, entry['text'].encode(), path.stat().st_mode & 0o777))
    before = {str(path): {'before': snapshot(path), 'installed': digest(data)} for path, data, _ in changes}
    enabled_before = run('systemctl', '--user', 'is-enabled', 'chuhshell.service', check=False).returncode == 0
    active_before = run('systemctl', '--user', 'is-active', 'chuhshell.service', check=False).returncode == 0
    standalone = None
    if not active_before and (pid := shell_pid()) is not None:
        try:
            standalone = str(Path(f'/proc/{pid}/exe').readlink())
        except OSError:
            pass
    journal = {'standalone': standalone, 'files': before, 'manifest': snapshot(MANIFEST), 'enabled': enabled_before, 'active': active_before, 'stopped': False}
    write(journal_path(), json.dumps(journal).encode(), 0o600)
    try:
        for path, data, mode in changes:
            save(path, data, mode, manifest)
        run('systemctl', '--user', 'daemon-reload')
        environment = [name for name in ['WAYLAND_DISPLAY', 'NIRI_SOCKET', 'DISPLAY', 'XDG_CURRENT_DESKTOP'] if name in os.environ]
        if environment:
            run('systemctl', '--user', 'import-environment', *environment)
        journal['stopped'] = True
        write(journal_path(), json.dumps(journal).encode(), 0o600)
        run('systemctl', '--user', 'stop', 'chuhshell.service')
        stop_old_shell()
        run('systemctl', '--user', 'enable', 'chuhshell.service')
        run('systemctl', '--user', 'reset-failed', 'chuhshell.service', check=False)
        run('systemctl', '--user', 'start', 'chuhshell.service')
        time.sleep(1)
        run('systemctl', '--user', 'is-active', 'chuhshell.service')
        if 'NIRI_SOCKET' in os.environ:
            run('niri', 'msg', 'action', 'load-config-file')
        write(MANIFEST, json.dumps(manifest, indent=2).encode(), 0o600)
        journal_path().unlink()
        sync_directory(STATE)
        print('Installed chuhshell and started its user service. Backups: ' + str(STATE))
    except BaseException:
        recover()
        raise


def uninstall():
    recover()
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
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=['install', 'uninstall'])
    parser.add_argument('--config', type=Path)
    args = parser.parse_args()
    try:
        STATE.mkdir(parents=True, exist_ok=True, mode=0o700)
        with (STATE / 'lock').open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            if args.action == 'install':
                install(args.config)
            else:
                uninstall()
    except (RuntimeError, OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f'chuhshell installation failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError):
            print(error.stderr, file=sys.stderr)
        sys.exit(1)
