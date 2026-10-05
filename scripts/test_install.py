import importlib.util
from pathlib import Path
import tempfile
import unittest
import json
import os
import storage
import subprocess
import sys
import time
import commands
from unittest.mock import patch
import autologin
from storage import snapshot, restore

spec = importlib.util.spec_from_file_location('installer', Path(__file__).with_name('install.py'))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class CommandBudgets(unittest.TestCase):
    def test_timeout_kills_descendants_and_inherited_pipes(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / 'survived'
            script = 'import subprocess, sys; subprocess.Popen([sys.executable, "-c", sys.argv[1]])'
            child = f'import time; from pathlib import Path; time.sleep(0.5); Path({str(marker)!r}).touch()'
            start = time.monotonic()
            with self.assertRaisesRegex(RuntimeError, 'timed out'):
                commands.run([sys.executable, '-c', script, child], timeout=0.15)
            self.assertLess(time.monotonic() - start, 1)
            time.sleep(0.6)
            self.assertFalse(marker.exists())

    def test_output_budget_checks_both_streams_even_without_check(self):
        for stream in ['1', '2']:
            with self.subTest(stream=stream), self.assertRaisesRegex(RuntimeError, 'output exceeds'):
                commands.run([sys.executable, '-c', f'import os; os.write({stream}, b"x" * 8192)'],
                             output_limit=1024, check=False)

    def test_input_and_exit_status_are_preserved(self):
        result = commands.run([sys.executable, '-c', 'import sys; sys.stdout.buffer.write(sys.stdin.buffer.read())'],
                              input=b'hello' * 10000)
        self.assertEqual(result.stdout, 'hello' * 10000)
        with self.assertRaises(subprocess.CalledProcessError) as error:
            commands.run([sys.executable, '-c', 'import sys; print("failed", file=sys.stderr); sys.exit(7)'])
        self.assertEqual(error.exception.returncode, 7)
        self.assertEqual(error.exception.stderr, 'failed\n')


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

    def test_uninstall_preserves_deletion_permissions_and_object_types(self):
        for change in ['delete', 'chmod', 'special-mode', 'link', 'dangling-link', 'replace-link']:
            with self.subTest(change=change):
                self.path.unlink(missing_ok=True)
                self.path.write_bytes(b'original')
                manifest = {}
                installer.save(self.path, b'installed', 0o644, manifest)
                if change == 'delete':
                    self.path.unlink()
                elif change in ['chmod', 'special-mode']:
                    self.path.chmod(0o600 if change == 'chmod' else 0o4644)
                else:
                    self.path.unlink()
                    target = self.root / ('missing' if change == 'dangling-link' else 'target')
                    if change != 'dangling-link':
                        target.write_bytes(b'installed')
                    self.path.symlink_to(target)
                    if change == 'replace-link':
                        self.path.unlink()
                        self.path.symlink_to(self.root / 'changed-target')
                before = snapshot(self.path)
                self.assertIn(str(self.path), installer.restore(manifest, True))
                self.assertEqual(snapshot(self.path), before)

    def test_dangling_original_link_is_restored(self):
        self.path.unlink()
        self.path.symlink_to(self.root / 'missing')
        original = snapshot(self.path)
        installer.save(self.path, b'installed', 0o644, self.manifest)
        self.assertEqual(installer.restore(self.manifest, True), {})
        self.assertEqual(snapshot(self.path), original)


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

    def test_service_uses_supported_environment_and_keeps_session_paths_dynamic(self):
        environment = {
            'XDG_CONFIG_HOME': str(self.root / 'custom config'),
            'XDG_DATA_HOME': str(self.root / 'custom data'),
            'XDG_STATE_HOME': str(self.root / 'custom state'),
            'XDG_CACHE_HOME': 'relative',
            'XDG_DATA_DIRS': '/one:relative:/two',
            'PATH': '/custom/bin:/usr/bin',
            'WAYLAND_DISPLAY': 'wayland-7',
            'NIRI_SOCKET': '/run/user/test/niri.sock',
            'LANG': 'en_US.UTF-8',
            'UNRELATED_SECRET': 'excluded',
        }
        commands = []
        def run(*args, **kwargs):
            commands.append(args)
            return self.original_run(*args, **kwargs)
        with patch.dict(os.environ, environment, clear=True), patch.object(installer, 'run', side_effect=run), patch.object(installer, 'plan_configuration', return_value=self.plan), patch.object(installer, 'stop_old_shell'), patch.object(installer.time, 'sleep'):
            installer.install(self.config)
            self.assertEqual(dict(os.environ), environment)
        service = (installer.CONFIG / 'systemd/user/chuhshell.service').read_text()
        for name in ('XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME', 'PATH', 'LANG'):
            self.assertIn(installer.unit_quote(name + '=' + environment[name]), service)
        self.assertIn(installer.unit_quote('XDG_CACHE_HOME=' + str(installer.HOME_DIR / '.cache')), service)
        self.assertIn('"XDG_DATA_DIRS=/one:/two"', service)
        self.assertIn('"XDG_CONFIG_DIRS=/etc/xdg"', service)
        self.assertNotIn('NIRI_SOCKET=', service)
        self.assertNotIn('WAYLAND_DISPLAY=', service)
        self.assertNotIn('UNRELATED_SECRET', service)
        imports = [command for command in commands if 'import-environment' in command]
        self.assertEqual(imports, [('systemctl', '--user', 'import-environment', 'WAYLAND_DISPLAY', 'NIRI_SOCKET')])
        self.assertEqual(installer.unit_quote('a%"\\\n'), '"a%%\\"\\\\\\x0a"')

    def test_process_exit_after_rename_recovers_installation(self):
        pid = os.fork()
        if pid == 0:
            original_sync = storage.sync_directory
            def interrupted(path):
                original_sync(path)
                if Path(path) == self.config.parent and self.config.read_text() == 'updated':
                    os._exit(78)
            with patch.object(storage, 'sync_directory', side_effect=interrupted):
                self.install()
            os._exit(79)
        _, status = os.waitpid(pid, 0)
        self.assertEqual(os.waitstatus_to_exitcode(status), 78)
        self.assertTrue(installer.journal_path().exists())
        with patch.object(installer, 'run', side_effect=self.original_run):
            installer.recover()
        self.assertEqual(self.config.read_text(), 'original')
        self.assertFalse(installer.journal_path().exists())
        self.install()
        with patch.object(installer, 'run', side_effect=self.original_run):
            installer.uninstall()

    def test_directory_sync_failure_after_profile_rename_rolls_back(self):
        original_sync = storage.sync_directory
        failed = False
        def interrupted(path):
            nonlocal failed
            if not failed and Path(path) == self.config.parent and self.config.read_text() == 'updated':
                failed = True
                raise OSError('Injected fsync failure after rename')
            return original_sync(path)
        with patch.object(storage, 'sync_directory', side_effect=interrupted):
            with self.assertRaises(storage.DurabilityError) as result:
                self.install()
        self.assertTrue(result.exception.committed)
        self.assertEqual(self.config.read_text(), 'original')
        self.assertFalse(installer.journal_path().exists())

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

    def test_timeout_during_install_and_recovery_keeps_journal_for_retry(self):
        def timeout(*args, **kwargs):
            if 'daemon-reload' in args:
                return commands.run([sys.executable, '-c', 'import time; time.sleep(5)'], timeout=0.05)
            return self.original_run(*args, **kwargs)
        with patch.object(installer, 'run', side_effect=timeout), patch.object(installer, 'plan_configuration', return_value=self.plan):
            with self.assertRaisesRegex(RuntimeError, 'timed out'):
                installer.install(self.config)
        self.assertTrue(installer.journal_path().exists())
        with patch.object(installer, 'run', side_effect=self.original_run):
            installer.recover()
        self.assertEqual(self.config.read_text(), 'original')
        self.assertFalse(installer.journal_path().exists())

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

    def test_uninstall_recovers_failures_before_and_after_every_step(self):
        original_write = installer.write
        original_restore = installer.restore_file
        for after in [False, True]:
            for fail in range(1, 32):
                with self.subTest(after=after, step=fail):
                    self.install()
                    calls = 0
                    def wrapped(operation):
                        def execute(*args, **kwargs):
                            nonlocal calls
                            calls += 1
                            if calls == fail and not after:
                                raise OSError('Interrupted uninstall')
                            result = operation(*args, **kwargs)
                            if calls == fail and after:
                                raise OSError('Interrupted uninstall after mutation')
                            return result
                        return execute
                    interrupted = False
                    with patch.object(installer, 'write', side_effect=wrapped(original_write)), patch.object(installer, 'restore_file', side_effect=wrapped(original_restore)), patch.object(installer, 'run', side_effect=wrapped(self.original_run)):
                        try:
                            installer.uninstall()
                        except OSError:
                            interrupted = True
                    if interrupted:
                        with patch.object(installer, 'run', side_effect=self.original_run):
                            installer.uninstall()
                    self.assertEqual(self.config.read_text(), 'original')
                    self.assertFalse((installer.HOME_DIR / '.local/bin/chuhshell').exists())
                    self.assertFalse(installer.MANIFEST.exists())
                    self.assertFalse(installer.journal_path().exists())

    def test_uninstall_resumes_after_manifest_removal_and_reload_failure(self):
        self.install()
        def run(*args, **kwargs):
            if 'daemon-reload' in args:
                self.assertFalse(installer.MANIFEST.exists())
                raise OSError('Interrupted reload')
            return self.original_run(*args, **kwargs)
        with patch.object(installer, 'run', side_effect=run):
            with self.assertRaises(OSError):
                installer.uninstall()
        self.assertTrue(installer.journal_path().exists())
        with patch.object(installer, 'run', side_effect=self.original_run):
            installer.uninstall()
        self.assertFalse(installer.journal_path().exists())

    def test_uninstall_preserves_edits_and_service_conflicts(self):
        self.install()
        service = installer.CONFIG / 'systemd/user/chuhshell.service'
        service.write_text('user service')
        self.config.unlink()
        commands = []
        def run(*args, **kwargs):
            commands.append(args)
            return self.original_run(*args, **kwargs)
        with patch.object(installer, 'run', side_effect=run):
            installer.uninstall()
        self.assertEqual(service.read_text(), 'user service')
        self.assertFalse(self.config.exists())
        self.assertFalse(any('disable' in args or 'start' in args or 'stop' in args for args in commands))
        manifest = json.loads(installer.MANIFEST.read_text())
        self.assertEqual(set(manifest), {str(service), str(self.config)})

    def test_service_original_state_survives_reinstall(self):
        for enabled, active in [(True, True), (True, False), (False, False), (False, True)]:
            with self.subTest(enabled=enabled, active=active):
                service = installer.CONFIG / 'systemd/user/chuhshell.service'
                service.parent.mkdir(parents=True, exist_ok=True)
                service.write_text('original service')
                active_queries = 0
                def initial_run(*args, **kwargs):
                    nonlocal active_queries
                    code = 0
                    if 'is-enabled' in args:
                        code = 0 if enabled else 1
                    if 'is-active' in args:
                        active_queries += 1
                        code = 0 if active or active_queries > 1 else 1
                    return subprocess.CompletedProcess(args, code, '', '')
                with patch.object(installer, 'run', side_effect=initial_run), patch.object(installer, 'plan_configuration', return_value=self.plan), patch.object(installer, 'stop_old_shell'), patch.object(installer, 'shell_pid', return_value=None), patch.object(installer.time, 'sleep'):
                    installer.install(self.config)
                updated_plan = [{'path': str(self.config), 'before': 'updated', 'text': 'updated'}]
                with patch.object(installer, 'run', side_effect=self.original_run), patch.object(installer, 'plan_configuration', return_value=updated_plan), patch.object(installer, 'stop_old_shell'), patch.object(installer.time, 'sleep'):
                    installer.install(self.config)
                manifest = json.loads(installer.MANIFEST.read_text())
                self.assertEqual(manifest[str(service)]['service_before'], {'enabled': enabled, 'active': active})
                commands = []
                def uninstall_run(*args, **kwargs):
                    commands.append(args)
                    return self.original_run(*args, **kwargs)
                with patch.object(installer, 'run', side_effect=uninstall_run):
                    installer.uninstall()
                self.assertEqual(service.read_text(), 'original service')
                self.assertIn(('systemctl', '--user', 'enable' if enabled else 'disable', 'chuhshell.service'), commands)
                self.assertIn(('systemctl', '--user', 'start' if active else 'stop', 'chuhshell.service'), commands)


class AutologinTransaction(unittest.TestCase):
    def test_managed_profile_ignores_comments_and_inactive_branches(self):
        for previous in [b'# exec niri --session\n', b'if false; then exec niri --session; fi\n']:
            content = autologin.profile_content(previous)
            self.assertTrue(content.startswith(autologin.BLOCK))
            self.assertTrue(content.endswith(previous))
            self.assertEqual(autologin.profile_content(content), content)
        with self.assertRaises(RuntimeError):
            autologin.profile_content(b'if broken\n')

    def test_profile_path_honors_zdotdir_and_zshenv(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            custom = home / 'zsh'
            custom.mkdir()
            (home / '.zshenv').write_text(f'ZDOTDIR={custom}\n')
            with patch.dict(os.environ, {'ZDOTDIR': ''}):
                self.assertEqual(autologin.profile_path(home), custom / '.zprofile')
            (home / '.zshenv').unlink()
            with patch.dict(os.environ, {'ZDOTDIR': str(custom)}):
                self.assertEqual(autologin.profile_path(home), custom / '.zprofile')

    def test_recovery_remembers_profile_when_zdotdir_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first, second, dropin = root / 'first', root / 'second', root / 'getty'
            first.write_text('original')
            second.write_text('unrelated')
            dropin.write_text('original getty')
            with patch.object(autologin, 'run', side_effect=lambda *args, **kwargs: subprocess.CompletedProcess(args, 0, '', '')), patch.object(autologin, 'privileged_restore', side_effect=restore):
                autologin.setup(root / 'backup', first, dropin, 'test-user')
                autologin.setup(root / 'backup', second, dropin, 'test-user', True)
            self.assertEqual(first.read_text(), 'original')
            self.assertEqual(second.read_text(), 'unrelated')
            self.assertEqual(dropin.read_text(), 'original getty')

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

    def test_autologin_timeout_and_failed_undo_keep_recovery_journal(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            profile, dropin = root / 'profile', root / 'getty'
            profile.write_text('original')
            def timeout(*args, **kwargs):
                if 'daemon-reload' in args:
                    return commands.run([sys.executable, '-c', 'import time; time.sleep(5)'], timeout=0.05)
                return subprocess.CompletedProcess(args, 0, '', '')
            with patch.object(autologin, 'run', side_effect=timeout), patch.object(autologin, 'privileged_restore', side_effect=restore):
                with self.assertRaisesRegex(RuntimeError, 'timed out'):
                    autologin.setup(root / 'backup', profile, dropin, 'test-user')
            self.assertTrue((root / 'backup/transaction.json').exists())
            with patch.object(autologin, 'run', return_value=subprocess.CompletedProcess([], 0)), patch.object(autologin, 'privileged_restore', side_effect=restore):
                autologin.setup(root / 'backup', profile, dropin, 'test-user', True)
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
