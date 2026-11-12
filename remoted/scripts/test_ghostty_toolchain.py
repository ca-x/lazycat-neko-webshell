import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location('ghostty_toolchain', Path(__file__).with_name('ghostty-toolchain.py'))
toolchain = importlib.util.module_from_spec(spec)
spec.loader.exec_module(toolchain)


class ToolchainTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.config = toolchain.pins()

    def test_all_desktop_runner_hosts_select_their_own_zig(self):
        for system, cpu, expected in [
            ('Linux', 'x86_64', 'x86_64-linux'),
            ('Windows', 'AMD64', 'x86_64-windows'),
            ('Darwin', 'arm64', 'aarch64-macos'),
            ('Darwin', 'x86_64', 'x86_64-macos'),
        ]:
            with self.subTest(system=system, cpu=cpu):
                self.assertEqual(toolchain.host_platform(system, cpu), expected)
        with self.assertRaisesRegex(ValueError, 'Unsupported'):
            toolchain.host_platform('Linux', 'riscv64')

    def test_binding_manifest_and_both_locked_crates_must_match_pin(self):
        (self.directory / 'ghostty').mkdir()
        (self.directory / 'client').mkdir()
        manifest = self.directory / 'ghostty/Cargo.toml'
        lock = self.directory / 'client/Cargo.lock'
        original_manifest = (toolchain.ROOT / 'ghostty/Cargo.toml').read_text()
        original_lock = (toolchain.ROOT / 'client/Cargo.lock').read_text()
        manifest.write_text(original_manifest)
        lock.write_text(original_lock)
        with patch.object(toolchain, 'ROOT', self.directory):
            self.assertEqual(toolchain.pins(), self.config)
            manifest.write_text(original_manifest.replace(self.config['binding_revision'], 'a' * 40))
            with self.assertRaisesRegex(ValueError, 'manifest'):
                toolchain.pins()
            manifest.write_text(original_manifest)
            lock.write_text(original_lock.replace(self.config['binding_revision'], 'b' * 40, 1))
            with self.assertRaisesRegex(ValueError, 'lockfile'):
                toolchain.pins()

    def test_corrupt_cached_archive_fails_before_network_or_extraction(self):
        archive = self.directory / 'source.tar.gz'
        archive.write_bytes(b'corrupt archive')
        with patch.object(toolchain.urllib.request, 'urlopen') as network:
            with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
                toolchain.download('https://example.invalid/source', archive, '0' * 64)
            network.assert_not_called()
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        self.assertEqual(toolchain.download('https://example.invalid/source', archive, digest), archive)

    def test_bad_download_digest_is_not_cached(self):
        archive = self.directory / 'source.tar.gz'
        with patch.object(toolchain.urllib.request, 'urlopen', return_value=io.BytesIO(b'wrong source')):
            with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
                toolchain.download('https://example.invalid/source', archive, '0' * 64)
        self.assertFalse(archive.exists())
        self.assertFalse(archive.with_suffix('.gz.partial').exists())

    def test_tar_and_windows_zip_install_one_root_and_replace_old_files(self):
        for suffix in ('.tar.xz', '.zip'):
            with self.subTest(suffix=suffix):
                archive = self.directory / ('zig' + suffix)
                if suffix == '.zip':
                    with zipfile.ZipFile(archive, 'w') as output:
                        output.writestr('zig-root/zig.exe', 'pinned zig')
                else:
                    with tarfile.open(archive, 'w:xz') as output:
                        entry = tarfile.TarInfo('zig-root/zig.exe')
                        entry.size = len(b'pinned zig')
                        output.addfile(entry, io.BytesIO(b'pinned zig'))
                destination = self.directory / 'installed'
                destination.mkdir(exist_ok=True)
                (destination / 'stale').write_text('old version')
                toolchain.unpack(archive, destination)
                self.assertEqual((destination / 'zig.exe').read_text(), 'pinned zig')
                self.assertFalse((destination / 'stale').exists())

    def test_zip_and_tar_path_traversal_are_rejected(self):
        archive = self.directory / 'bad.zip'
        with zipfile.ZipFile(archive, 'w') as output:
            output.writestr('../escaped', 'bad')
        with self.assertRaises(ValueError):
            toolchain.unpack(archive, self.directory / 'installed')
        archive = self.directory / 'bad.tar.gz'
        with tarfile.open(archive, 'w:gz') as output:
            entry = tarfile.TarInfo('../escaped')
            entry.size = 3
            output.addfile(entry, io.BytesIO(b'bad'))
        with self.assertRaises(tarfile.FilterError):
            toolchain.unpack(archive, self.directory / 'installed')
        self.assertFalse((self.directory / 'escaped').exists())

    def test_setup_verifies_local_archives_and_exports_only_install_location(self):
        config = dict(self.config)
        config['zig_archives'] = {'x86_64-windows': {'suffix': '.zip'}}
        cache = self.directory / 'cache'
        cache.mkdir()
        zig = cache / f"zig-x86_64-windows-{config['zig_version']}.zip"
        with zipfile.ZipFile(zig, 'w') as output:
            output.writestr('zig-root/zig.exe', 'compiler fixture')
        config['zig_archives']['x86_64-windows']['sha256'] = toolchain.checksum(zig)
        source = cache / f"ghostty-{config['ghostty_revision']}.tar.gz"
        with tarfile.open(source, 'w:gz') as output:
            entry = tarfile.TarInfo('ghostty-root/build.zig')
            entry.size = 6
            output.addfile(entry, io.BytesIO(b'source'))
        config['ghostty_archive_sha256'] = toolchain.checksum(source)
        installed = self.directory / 'toolchain'
        github_env = self.directory / 'github-env'
        with patch.object(toolchain, 'host_platform', return_value='x86_64-windows'), \
             patch.object(toolchain, 'verify') as verify, \
             patch.object(toolchain.urllib.request, 'urlopen') as network, \
             patch('sys.stdout', new_callable=io.StringIO), \
             patch.dict(os.environ, {'GITHUB_ENV': str(github_env)}):
            toolchain.setup(installed, cache, config)
            verify.assert_called_once_with(installed, config)
            network.assert_not_called()
        self.assertEqual((installed / 'source/build.zig').read_text(), 'source')
        self.assertEqual((installed / 'zig/zig.exe').read_text(), 'compiler fixture')
        self.assertEqual(json.loads((installed / 'pins.json').read_text()), config)
        self.assertEqual(github_env.read_text(), f'NEKO_GHOSTTY_TOOLCHAIN={installed}\n')

    def test_run_rejects_wrong_installed_pin_or_actual_zig_version(self):
        (self.directory / 'source').mkdir()
        (self.directory / 'source/build.zig').write_text('source')
        record = self.directory / 'pins.json'
        record.write_text('{}')
        with self.assertRaisesRegex(ValueError, 'Installed toolchain differs'):
            toolchain.verify(self.directory, self.config)
        record.write_text(json.dumps(self.config))
        with patch.object(toolchain.subprocess, 'check_output', return_value='0.16.0\n'):
            with self.assertRaisesRegex(ValueError, 'Zig version differs'):
                toolchain.verify(self.directory, self.config)

    def test_child_process_has_no_docs_rs_and_uses_pinned_source_and_zig(self):
        with patch.object(toolchain, 'verify'), patch.dict(os.environ, {'DOCS_RS': '1', 'GHOSTTY_ZIG_SYSTEM_DIR': 'stale'}):
            environment = toolchain.build_environment(self.directory, self.config)
        observed = json.loads(subprocess.check_output([
            sys.executable, '-c',
            'import json,os; print(json.dumps({k:os.environ.get(k) for k in '
            '["DOCS_RS","GHOSTTY_ZIG_SYSTEM_DIR","GHOSTTY_SOURCE_DIR","ZIG","LIBGHOSTTY_VT_SYS_CPU"]}))',
        ], env=environment, text=True))
        self.assertIsNone(observed['DOCS_RS'])
        self.assertIsNone(observed['GHOSTTY_ZIG_SYSTEM_DIR'])
        self.assertEqual(observed['GHOSTTY_SOURCE_DIR'], str(self.directory / 'source'))
        self.assertEqual(observed['ZIG'], str(toolchain.executable(self.directory)))
        self.assertEqual(observed['LIBGHOSTTY_VT_SYS_CPU'], 'baseline')

    def test_android_uses_api28_wrapper_and_both_pinned_ndk_linkers(self):
        ndk = self.directory / 'ndk'
        ndk.mkdir()
        properties = ndk / 'source.properties'
        properties.write_text(f"Pkg.Revision = {self.config['android_ndk']}\n")
        tool_bin = ndk / 'toolchains/llvm/prebuilt/linux-x86_64/bin'
        tool_bin.mkdir(parents=True)
        for cpu in ('aarch64', 'x86_64'):
            (tool_bin / f'{cpu}-linux-android28-clang').touch()
        with patch.object(toolchain, 'verify'), patch.object(toolchain.platform, 'system', return_value='Linux'), patch.dict(os.environ, {'ANDROID_NDK_HOME': str(ndk)}):
            environment = toolchain.build_environment(self.directory, self.config, android=True)
            self.assertEqual(environment['ZIG'], str(toolchain.ROOT / 'ghostty/tools/zig-android.sh'))
            self.assertEqual(environment['NEKO_GHOSTTY_ANDROID_API'], '28')
            for cpu in ('aarch64', 'x86_64'):
                self.assertEqual(environment[f'CARGO_TARGET_{cpu.upper()}_LINUX_ANDROID_LINKER'], str(tool_bin / f'{cpu}-linux-android28-clang'))
            properties.write_text('Pkg.Revision = 26.0.0\n')
            with self.assertRaisesRegex(ValueError, 'NDK differs'):
                toolchain.build_environment(self.directory, self.config, android=True)

    @unittest.skipIf(sys.platform == 'win32', 'Android wrapper runs on Linux')
    def test_android_wrapper_rewrites_only_android_targets_and_preserves_other_arguments(self):
        # Use Python as a real executable to observe argv after Bash rewrites it.
        arguments = ['build', '-Dtarget=aarch64-linux-android', '-Dtarget=x86_64-linux-android',
                     '-Dtarget=x86_64-windows-msvc', '--prefix', 'path with spaces']
        environment = dict(os.environ, NEKO_GHOSTTY_ZIG=sys.executable, NEKO_GHOSTTY_ANDROID_API='28')
        command = ['bash', str(toolchain.ROOT / 'ghostty/tools/zig-android.sh'), '-c',
                   'import json,sys; print(json.dumps(sys.argv[1:]))', *arguments]
        observed = json.loads(subprocess.check_output(command, env=environment, text=True))
        self.assertEqual(observed, ['build', '-Dtarget=aarch64-linux-android.28',
                                   '-Dtarget=x86_64-linux-android.28', *arguments[3:]])


if __name__ == '__main__':
    unittest.main()
