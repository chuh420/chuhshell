import base64
import os
from pathlib import Path
import tempfile


def sync_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def write(path, data, mode=0o600):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix='.' + path.name + '-', dir=path.parent)
    temporary = Path(name)
    try:
        with os.fdopen(fd, 'wb') as stream:
            os.fchmod(stream.fileno(), mode)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        temporary.replace(path)
        sync_directory(path.parent)
    finally:
        temporary.unlink(missing_ok=True)


def snapshot(path):
    if path.is_symlink():
        return {'link': os.readlink(path)}
    if not path.exists():
        return None
    return {'data': base64.b64encode(path.read_bytes()).decode(), 'mode': path.stat().st_mode & 0o777}


def restore(path, entry):
    if entry is None:
        path.unlink(missing_ok=True)
    elif 'link' in entry:
        if path.is_symlink() and os.readlink(path) == entry['link']:
            return
        fd, name = tempfile.mkstemp(prefix='.' + path.name + '-', dir=path.parent)
        os.close(fd)
        os.unlink(name)
        try:
            os.symlink(entry['link'], name)
            os.replace(name, path)
        finally:
            Path(name).unlink(missing_ok=True)
    else:
        write(path, base64.b64decode(entry['data']), entry['mode'])
    if path.parent.exists():
        sync_directory(path.parent)
