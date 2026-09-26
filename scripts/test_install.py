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


class MenuBindings(unittest.TestCase):
    def test_replaces_bindings_and_preserves_other_actions(self):
        source = '\n'.join([
            'binds {',
            '    Mod+D { spawn "chuhshell" "launcher"; }',
            '    Mod+Space { spawn "old-menu"; }',
            '    Mod+Shift+D {',
            '        spawn "chuhshell" "manage"',
            '    }',
            '    Alt+F4 { close-window; }',
            '}',
        ])
        result = installer.menu_bindings(source, Path('/home/test/.local/bin/chuhshell'))
        self.assertNotIn('Mod+Shift+D', result)
        self.assertNotIn('old-menu', result)
        self.assertIn('Mod+D { spawn "chuhshell" "launcher"; }', result)
        self.assertIn('Alt+F4 { close-window; }', result)
        self.assertEqual(result.count('Mod+Space'), 1)
        self.assertEqual(installer.menu_bindings(result, Path('/home/test/.local/bin/chuhshell')), result)

    def test_adds_menu_binding(self):
        result = installer.menu_bindings('binds {\n}\n', Path('/usr/bin/chuhshell'))
        self.assertIn('spawn "/usr/bin/chuhshell" "menu"', result)


if __name__ == '__main__':
    unittest.main()
