import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('installer', Path(__file__).with_name('install.py'))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallationBackups(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        installer.STATE = self.root / 'backups'
        installer.STATE.mkdir()
        self.path = self.root / 'existing.conf'
        self.path.write_bytes(b'original')
        self.manifest = {}

    def tearDown(self):
        self.directory.cleanup()

    def test_repeated_install_keeps_original_backup(self):
        installer.save(self.path, b'installed', 0o644, self.manifest)
        installer.save(self.path, b'updated', 0o644, self.manifest)
        self.assertEqual(installer.restore(self.manifest, True), {})
        self.assertEqual(self.path.read_bytes(), b'original')

    def test_uninstall_preserves_later_user_edits(self):
        installer.save(self.path, b'installed', 0o644, self.manifest)
        self.path.write_bytes(b'user changes')
        self.assertIn(str(self.path), installer.restore(self.manifest, True))
        self.assertEqual(self.path.read_bytes(), b'user changes')

    def test_uninstall_removes_only_created_files(self):
        created = self.root / 'created.conf'
        installer.save(created, b'new', 0o600, self.manifest)
        self.assertEqual(installer.restore(self.manifest, True), {})
        self.assertFalse(created.exists())
        self.assertEqual(self.path.read_bytes(), b'original')


if __name__ == '__main__':
    unittest.main()
