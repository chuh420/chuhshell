import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pwd
import shutil
import subprocess
import sys
import tempfile
import uuid
from storage import write, snapshot, restore, sync_directory


def run(*args, check=True):
    return subprocess.run(args, check=check, capture_output=True, text=True)


def privileged_restore(path, entry):
    print('Using sudo to restore the tty1 getty configuration.')
    if entry is None:
        run('sudo', 'rm', '-f', str(path))
    elif 'link' in entry:
        temporary = path.with_name('.chuhshell-' + uuid.uuid4().hex)
        run('sudo', 'ln', '-s', '--', entry['link'], str(temporary))
        run('sudo', 'mv', '-Tf', '--', str(temporary), str(path))
    else:
        import base64
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'dropin'
            write(source, base64.b64decode(entry['data']), 0o600)
            run('sudo', 'mkdir', '-p', str(path.parent))
            temporary = path.with_name('.chuhshell-' + uuid.uuid4().hex)
            try:
                run('sudo', 'install', '-m', format(entry['mode'], 'o'), str(source), str(temporary))
                run('sudo', 'mv', '-Tf', str(temporary), str(path))
            finally:
                run('sudo', 'rm', '-f', str(temporary), check=False)
    if path.parent.exists():
        run('sudo', 'sync', '-f', str(path.parent))


def checksum(path):
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.exists() else None


def undo(state, profile, dropin, record):
    for path, key in [(profile, 'profile'), (dropin, 'dropin')]:
        if snapshot(path) != record[key] and checksum(path) != record['expected'][key]:
            raise RuntimeError(f'{path} changed since setup; preserve the edits before restoring {state}')
    restore(profile, record['profile'])
    privileged_restore(dropin, record['dropin'])
    print('Using sudo to restore getty service settings.')
    run('sudo', 'systemctl', 'daemon-reload')
    run('sudo', 'systemctl', 'enable' if record['enabled'] else 'disable', 'getty@tty1.service')
    state.unlink()
    sync_directory(state.parent)


def setup(directory, profile, dropin, username, undo_requested=False):
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    state = directory / 'transaction.json'
    if state.exists():
        record = json.loads(state.read_text())
        if undo_requested or record['phase'] != 'installed':
            undo(state, profile, dropin, record)
            print('Restored the previous login configuration.')
            if undo_requested:
                return
        else:
            print('Autologin is already managed by this script.')
            return
    elif (directory / 'installed').exists():
        if not undo_requested:
            print('Autologin is already managed by the previous installer. Use --undo before changing it.')
            return
        for key, path in [('profile', profile), ('dropin', dropin)]:
            hashfile = directory / (key + '.sha256')
            if hashfile.exists() and checksum(path) != hashfile.read_text().split()[0]:
                raise RuntimeError(f'{path} changed since setup; preserve the edits before restoring')
        restore(profile, snapshot(directory / 'profile'))
        privileged_restore(dropin, snapshot(directory / 'dropin'))
        print('Using sudo to reload getty configuration.')
        run('sudo', 'systemctl', 'daemon-reload')
        (directory / 'installed').unlink()
        return
    elif undo_requested:
        raise RuntimeError('No autologin backup found')
    previous = profile.read_bytes() if profile.exists() else b''
    block = b'\nif [[ -o interactive && "$TTY" == /dev/tty1 && -z "$WAYLAND_DISPLAY" && -z "$DISPLAY" ]]; then\n    exec niri --session\nfi\n'
    content = previous if b'exec niri --session' in previous else previous + block
    getty = f'[Service]\nExecStart=\nExecStart=-/usr/bin/agetty --autologin {username} --noclear %I $TERM\n'.encode()
    record = {'phase': 'prepared', 'profile': snapshot(profile), 'dropin': snapshot(dropin),
              'enabled': run('systemctl', 'is-enabled', 'getty@tty1.service', check=False).returncode == 0,
              'expected': {'profile': hashlib.sha256(content).hexdigest(), 'dropin': hashlib.sha256(getty).hexdigest()}}
    write(state, json.dumps(record).encode())
    try:
        import base64
        privileged_restore(dropin, {'data': base64.b64encode(getty).decode(), 'mode': 0o644})
        write(profile, content, profile.stat().st_mode & 0o777 if profile.exists() else 0o644)
        print('Using sudo to reload and enable the tty1 getty service.')
        run('sudo', 'systemctl', 'daemon-reload')
        run('sudo', 'systemctl', 'enable', 'getty@tty1.service')
        record['phase'] = 'installed'
        write(state, json.dumps(record).encode())
    except BaseException:
        undo(state, profile, dropin, record)
        raise
    print('Configured tty1 autologin. Reboot to activate; use --undo to restore backups.')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--undo', action='store_true')
    args = parser.parse_args()
    if os.geteuid() == 0:
        raise RuntimeError('Run this script as the desktop user; it invokes sudo where needed.')
    account = pwd.getpwuid(os.getuid())
    home = Path(account.pw_dir)
    if not account.pw_shell.endswith('/zsh') or not home.is_dir():
        raise RuntimeError('This setup requires a zsh login account with an existing home directory.')
    if not args.undo and shutil.which('niri') is None:
        raise RuntimeError('Install Niri first')
    directory = Path(os.environ.get('XDG_STATE_HOME') or home / '.local/state') / 'chuhshell/autologin-backup'
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    with (directory / 'lock').open('w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        setup(directory, home / '.zprofile', Path('/etc/systemd/system/getty@tty1.service.d/20-autologin.conf'), account.pw_name, args.undo)


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f'Autologin setup failed: {error}', file=sys.stderr)
        sys.exit(1)
