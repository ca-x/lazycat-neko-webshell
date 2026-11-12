import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import native_build_inputs as inputs


class NativeBuildInputsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def test_dirty_scope_detects_each_shared_source_and_ignores_unrelated_files(self):
        repo = self.root / 'repo'
        repo.mkdir()
        def git(*args):
            return subprocess.check_output(['git', '-C', str(repo), *args], stderr=subprocess.DEVNULL)
        git('init')
        ghostty = repo / 'remoted/ghostty'
        ghostty.mkdir(parents=True)
        shutil.copyfile(inputs.ROOT / 'ghostty/toolchain.json', ghostty / 'toolchain.json')
        paths = ['remoted/client/a.rs', 'remoted/protocol/a.rs', 'remoted/host/a.rs',
                 'remoted/themes/catalog.json', 'remoted/ghostty/a.rs',
                 'crates/hc-client/a.rs', 'crates/program-status/a.rs', 'remoted/scripts/build.py']
        for name in paths:
            path = repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('original')
        git('add', '.')
        git('-c', 'user.name=Build test', '-c', 'user.email=build@example.invalid', 'commit', '-m', 'fixture')
        with patch.object(inputs, 'ROOT', repo / 'remoted'):
            self.assertFalse(inputs.metadata()['source_dirty'])
            (repo / 'unrelated.txt').write_text('unrelated')
            self.assertFalse(inputs.metadata()['source_dirty'])
            for name in paths:
                with self.subTest(name=name):
                    (repo / name).write_text('modified')
                    self.assertTrue(inputs.metadata()['source_dirty'])
                    (repo / name).write_text('original')
            (repo / 'remoted/themes/new.json').write_text('{}')
            self.assertTrue(inputs.metadata()['source_dirty'])

    def test_apk_and_desktop_zip_preserve_every_pinned_notice(self):
        for prefix in ['assets', '.', 'neoko-terminal.app/Contents/Resources']:
            with self.subTest(prefix=prefix):
                stage = self.root / 'stage'
                inputs.stage_licenses(stage / 'licenses')
                archive = self.root / 'package.zip'
                with zipfile.ZipFile(archive, 'w') as output:
                    for file in stage.rglob('*'):
                        if file.is_file():
                            name = file.relative_to(stage).as_posix()
                            output.write(file, name if prefix == '.' else prefix + '/' + name)
                inputs.verify_zip(archive, prefix)

    def test_corrupt_or_missing_packaged_notice_fails_verification(self):
        archive = self.root / 'app.apk'
        files = inputs.license_files()
        for corruption in ('missing', 'modified'):
            with self.subTest(corruption=corruption), zipfile.ZipFile(archive, 'w') as output:
                for name, data in files.items():
                    if name == 'GHOSTTY-LICENSE':
                        if corruption == 'missing':
                            continue
                        data = b'truncated license'
                    output.writestr('assets/licenses/ghostty/' + name, data)
            with self.assertRaisesRegex(ValueError, 'Packaged Ghostty license differs'):
                inputs.verify_zip(archive, 'assets')

    def test_stale_or_modified_vendored_licenses_fail_before_copy(self):
        shutil.copytree(inputs.ROOT / 'ghostty', self.root / 'ghostty', ignore=shutil.ignore_patterns('target'))
        with patch.object(inputs, 'ROOT', self.root):
            (self.root / 'ghostty/licenses/GHOSTTY-LICENSE').write_text('modified')
            with self.assertRaisesRegex(ValueError, 'digest differs'):
                inputs.stage_licenses(self.root / 'output')
            self.assertFalse((self.root / 'output').exists())

    def test_metadata_merge_preserves_existing_provenance_inputs(self):
        build = self.root / 'build-info.json'
        metadata = self.root / 'build-inputs.json'
        original = {'apk_sha256': 'a' * 64, 'certificate_sha256': 'b' * 64,
                    'zip_alignment_verified': True, 'elf_load_alignment_min': 16384}
        build.write_text(json.dumps(original))
        actual = inputs.metadata()
        metadata.write_text(json.dumps(actual))
        subprocess.run([sys.executable, str(Path(inputs.__file__)), 'merge-metadata', str(build), str(metadata)], check=True)
        merged = json.loads(build.read_text())
        self.assertEqual({key: merged[key] for key in original}, original)
        self.assertEqual(merged['ghostty_source_revision'], actual['ghostty_source_revision'])
        self.assertEqual(merged['ghostty_wrapper_revision'], actual['ghostty_wrapper_revision'])

    def desktop_package(self, system, target):
        spec = importlib.util.spec_from_file_location('desktop_package', inputs.ROOT / 'client/scripts/build-desktop.py')
        desktop = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(desktop)
        target_dir = self.root / 'target'
        binary = target_dir / target / 'release' / ('neoko-terminal.exe' if system == 'Windows' else 'neoko-terminal')
        binary.parent.mkdir(parents=True)
        if system == 'Linux':
            shutil.copyfile('/usr/bin/true', binary)
        else:
            binary.write_bytes(b'MZ packaging fixture; not an application build')
        original_run = desktop.run
        def package_run(*args, **kwargs):
            if args[0] == 'cargo':
                return subprocess.CompletedProcess(args, 0)
            return original_run(*args, **kwargs)
        destination = self.root / 'dist'
        with patch.object(desktop.platform, 'system', return_value=system), \
             patch.object(desktop, 'run', side_effect=package_run), \
             patch.dict(os.environ, {'CARGO_TARGET_DIR': str(target_dir)}), \
             patch.object(sys, 'argv', ['build-desktop.py', '--version', '0.4.0', '--target', target, '--out', str(destination)]), \
             patch('sys.stdout'):
            desktop.main()
        return destination

    def test_windows_archive_contains_licenses_and_matching_inner_outer_metadata(self):
        destination = self.desktop_package('Windows', 'x86_64-pc-windows-msvc')
        archive = next(destination.glob('*.zip'))
        inputs.verify_zip(archive, '.')
        outer = json.loads(next(destination.glob('*.build-info.json')).read_text())
        with zipfile.ZipFile(archive) as package:
            self.assertEqual(json.loads(package.read('build-info.json')), outer)
        self.assertEqual(outer['desktop_version'], '0.4.0')
        self.assertIn('ghostty_source_revision', outer)
        self.assertIn('ghostty_wrapper_revision', outer)

    @unittest.skipUnless(sys.platform == 'linux' and shutil.which('dpkg-deb'), 'Requires Linux dpkg-deb')
    def test_real_deb_preserves_notices_and_build_metadata(self):
        destination = self.desktop_package('Linux', 'x86_64-unknown-linux-gnu')
        package = next(destination.glob('*.deb'))
        extracted = self.root / 'extracted'
        subprocess.run(['dpkg-deb', '--extract', str(package), str(extracted)], check=True)
        docs = extracted / 'usr/share/doc/neoko-terminal'
        for name, data in inputs.license_files().items():
            self.assertEqual((docs / 'licenses/ghostty' / name).read_bytes(), data)
        self.assertEqual(json.loads((docs / 'build-info.json').read_text()),
                         json.loads(next(destination.glob('*.build-info.json')).read_text()))


if __name__ == '__main__':
    unittest.main()
