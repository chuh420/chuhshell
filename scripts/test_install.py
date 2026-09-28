import importlib.util
from pathlib import Path
import tempfile
import unittest
import json
import subprocess
from unittest.mock import patch
import autologin
from storage import snapshot, restore

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

    def test_install_and_restore_preserve_symbolic_links(self):
        target = self.root / 'target'
        target.write_bytes(b'original link target')
        link = self.root / 'link'
        link.symlink_to(target)
        installer.save(link, b'installed', 0o644, self.manifest)
        self.assertFalse(link.is_symlink())
        self.assertEqual(installer.restore(self.manifest, True), {})
        self.assertTrue(link.is_symlink())
        self.assertEqual(target.read_bytes(), b'original link target')

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


class InstallationTransaction(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.patches = []
        for name, value in [('ROOT', self.root / 'repo'), ('HOME_DIR', self.root / 'home'), ('CONFIG', self.root / 'config'), ('DATA', self.root / 'data'), ('STATE', self.root / 'state'), ('MANIFEST', self.root / 'state/manifest.json')]:
            mock = patch.object(installer, name, value)
            mock.start()
            self.patches.append(mock)
        (installer.ROOT / 'target/release').mkdir(parents=True)
        (installer.ROOT / 'target/release/chuhshell').write_bytes(b'binary')
        (installer.ROOT / 'packaging').mkdir()
        for name in ['chuhshell.service', 'org.freedesktop.Notifications.service']:
            (installer.ROOT / 'packaging' / name).write_text('ExecStart=/usr/bin/chuhshell')
        self.config = self.root / 'active.kdl'
        self.config.write_text('original')
        self.plan = [{'path': str(self.config), 'before': 'original', 'text': 'updated'}]
        self.original_run = lambda *args, **kwargs: subprocess.CompletedProcess(args, 0, '', '')

    def tearDown(self):
        for mock in reversed(self.patches):
            mock.stop()
        self.directory.cleanup()

    def install(self, fail=None):
        count = 0
        def run(*args, **kwargs):
            nonlocal count
            count += 1
            if count == fail:
                raise RuntimeError('Injected failure')
            return self.original_run(*args, **kwargs)
        with patch.object(installer, 'run', side_effect=run), patch.object(installer, 'plan_configuration', return_value=self.plan), patch.object(installer, 'stop_old_shell'), patch.object(installer.time, 'sleep'):
            installer.install(self.config)
        return count

    def test_every_command_failure_restores_original_files(self):
        for fail in range(1, 12):
            with self.subTest(step=fail):
                try:
                    self.install(fail)
                except RuntimeError:
                    self.assertEqual(self.config.read_text(), 'original')
                    self.assertFalse((installer.HOME_DIR / '.local/bin/chuhshell').exists())
                    self.assertFalse(installer.MANIFEST.exists())
                    self.assertFalse(installer.journal_path().exists())
                else:
                    with patch.object(installer, 'run', side_effect=self.original_run):
                        installer.uninstall()

    def test_each_atomic_write_failure_rolls_back(self):
        original_write = installer.write
        for fail in range(1, 12):
            with self.subTest(write=fail):
                calls = 0
                def write(*args, **kwargs):
                    nonlocal calls
                    calls += 1
                    if calls == fail:
                        raise OSError('Injected disk error')
                    return original_write(*args, **kwargs)
                with patch.object(installer, 'write', side_effect=write):
                    try:
                        self.install()
                    except OSError:
                        self.assertEqual(self.config.read_text(), 'original')
                        self.assertFalse((installer.HOME_DIR / '.local/bin/chuhshell').exists())
                        self.assertFalse(installer.MANIFEST.exists())
                    else:
                        with patch.object(installer, 'run', side_effect=self.original_run), patch.object(installer, 'write', side_effect=original_write):
                            installer.uninstall()

    def test_interrupted_install_recovers_before_retry(self):
        installer.STATE.mkdir()
        before = snapshot(self.config)
        self.config.write_text('updated')
        journal = {'files': {str(self.config): {'before': before, 'installed': installer.digest(b'updated')}}, 'manifest': None, 'enabled': False, 'active': False, 'stopped': False}
        installer.write(installer.journal_path(), json.dumps(journal).encode(), 0o600)
        self.install()
        manifest = json.loads(installer.MANIFEST.read_text())
        self.assertEqual(Path(manifest[str(self.config)]['backup']).read_text(), 'original')

    def test_stale_configuration_is_rejected_before_writes(self):
        self.config.write_text('user changes')
        with self.assertRaises(RuntimeError):
            self.install()
        self.assertEqual(self.config.read_text(), 'user changes')
        self.assertFalse((installer.HOME_DIR / '.local/bin/chuhshell').exists())

    def test_user_edits_after_interruption_are_preserved(self):
        installer.STATE.mkdir()
        journal = {'files': {str(self.config): {'before': snapshot(self.config), 'installed': installer.digest(b'updated')}}, 'manifest': None, 'enabled': False, 'active': False, 'stopped': False}
        installer.write(installer.journal_path(), json.dumps(journal).encode(), 0o600)
        self.config.write_text('user changes')
        with self.assertRaises(RuntimeError):
            installer.recover()
        self.assertEqual(self.config.read_text(), 'user changes')
        self.assertTrue(installer.journal_path().exists())

    def test_deleted_file_after_interruption_is_preserved(self):
        installer.STATE.mkdir()
        journal = {'files': {str(self.config): {'before': snapshot(self.config), 'installed': installer.digest(b'updated')}}, 'manifest': None, 'enabled': False, 'active': False, 'stopped': False}
        installer.write(installer.journal_path(), json.dumps(journal).encode(), 0o600)
        self.config.unlink()
        with self.assertRaises(RuntimeError):
            installer.recover()
        self.assertFalse(self.config.exists())
        self.assertTrue(installer.journal_path().exists())


class AutologinTransaction(unittest.TestCase):
    def test_failures_restore_profile_and_getty_and_allow_retry(self):
        for fail in range(1, 4):
            with self.subTest(step=fail), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                profile, dropin = root / 'profile', root / 'getty'
                profile.write_text('original profile')
                dropin.write_text('original getty')
                count = 0
                def run(*args, **kwargs):
                    nonlocal count
                    count += 1
                    if count == fail:
                        raise RuntimeError('Injected failure')
                    return subprocess.CompletedProcess(args, 0, '', '')
                with patch.object(autologin, 'run', side_effect=run), patch.object(autologin, 'privileged_restore', side_effect=restore):
                    try:
                        autologin.setup(root / 'backup', profile, dropin, 'test-user')
                    except RuntimeError:
                        self.assertEqual(profile.read_text(), 'original profile')
                        self.assertEqual(dropin.read_text(), 'original getty')
                    else:
                        autologin.setup(root / 'backup', profile, dropin, 'test-user', True)
                        self.assertEqual(profile.read_text(), 'original profile')
                        self.assertEqual(dropin.read_text(), 'original getty')

    def test_profile_write_failure_restores_privileged_change(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            profile, dropin = root / 'profile', root / 'getty'
            profile.write_text('original')
            original_write = autologin.write
            def write(path, *args, **kwargs):
                if path == profile:
                    raise OSError('Injected profile write error')
                return original_write(path, *args, **kwargs)
            with patch.object(autologin, 'write', side_effect=write), patch.object(autologin, 'run', return_value=subprocess.CompletedProcess([], 0)), patch.object(autologin, 'privileged_restore', side_effect=restore):
                with self.assertRaises(OSError):
                    autologin.setup(root / 'backup', profile, dropin, 'test-user')
            self.assertEqual(profile.read_text(), 'original')
            self.assertFalse(dropin.exists())
            self.assertFalse((root / 'backup/transaction.json').exists())

    def test_interrupted_setup_can_be_undone(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            profile, dropin = root / 'profile', root / 'getty'
            profile.write_text('original')
            def fail(*args):
                raise SystemExit('Power loss')
            with patch.object(autologin, 'run', return_value=subprocess.CompletedProcess([], 0)), patch.object(autologin, 'privileged_restore', side_effect=fail):
                with self.assertRaises(SystemExit):
                    autologin.setup(root / 'backup', profile, dropin, 'test-user')
            self.assertTrue((root / 'backup/transaction.json').exists())
            with patch.object(autologin, 'run', return_value=subprocess.CompletedProcess([], 0)), patch.object(autologin, 'privileged_restore', side_effect=restore):
                autologin.setup(root / 'backup', profile, dropin, 'test-user', True)
            self.assertEqual(profile.read_text(), 'original')
            self.assertFalse(dropin.exists())


if __name__ == '__main__':
    unittest.main()
