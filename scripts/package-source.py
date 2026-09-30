import argparse
import gzip
import hashlib
from pathlib import Path
import tarfile
import tomllib


def create(destination):
    root = Path(__file__).resolve().parent.parent
    metadata = tomllib.loads((root / 'Cargo.toml').read_text())['package']
    name = f"{metadata['name']}-{metadata['version']}"
    destination.mkdir(parents=True, exist_ok=True)
    archive = destination / (name + '.tar.gz')
    files = [root / filename for filename in ['Cargo.toml', 'Cargo.lock', 'README.md', 'config.example.json']]
    for folder in ['src', 'scripts', 'packaging']:
        files.extend(path for path in (root / folder).rglob('*') if path.is_file() and '__pycache__' not in path.parts and path.suffix in ['.rs', '.py', '.sh', '.service'])
    with archive.open('wb') as raw, gzip.GzipFile(fileobj=raw, mode='wb', filename='', mtime=0) as compressed, tarfile.open(fileobj=compressed, mode='w') as tar:
        for path in sorted(files):
            info = tar.gettarinfo(str(path), arcname=name + '/' + str(path.relative_to(root)))
            info.uid = info.gid = info.mtime = 0
            info.uname = info.gname = ''
            info.mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
            with path.open('rb') as stream:
                tar.addfile(info, stream)
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    recipe = (root / 'packaging/PKGBUILD').read_text().replace('SOURCE_ARCHIVE_CHECKSUM', checksum)
    (destination / 'PKGBUILD').write_text(recipe)
    print(archive)
    print('sha256: ' + checksum)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('destination', type=Path)
    create(parser.parse_args().destination)
