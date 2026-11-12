import configparser
import ctypes.util
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile


spec = importlib.util.spec_from_file_location('build_desktop', Path(__file__).resolve().parents[1] / 'client/scripts/build-desktop.py')
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)
CLIENT = builder.CLIENT
DPKG_DEB = os.environ.get('DPKG_DEB', shutil.which('dpkg-deb'))
SDK = Path(os.environ.get('ANDROID_HOME', os.environ.get('ANDROID_SDK_ROOT', str(Path.home() / 'Android/Sdk'))))
AAPT2 = SDK / 'build-tools/35.0.1/aapt2'
ANDROID_JAR = SDK / 'platforms/android-35/android.jar'


class DesktopPackageNameTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.client = self.root / 'client'
        self.client.mkdir()
        (self.root / 'LICENSE').write_bytes((CLIENT.parent / 'LICENSE').read_bytes())
        for name in ('android/licenses/NSH-LICENSE-APACHE', 'android/licenses/NOTICE',
                     'android/res/drawable/neko.png'):
            destination = self.client / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes((CLIENT / name).read_bytes())
        (self.client / 'Cargo.toml').write_text('[package]\nversion = "0.3.0"\n')
        (self.client / 'Cargo.lock').write_text(builder.NSH_REV)
        self.payload = b'native executable packaging fixture\n'
        self.signed_apps = []

    def build(self, system, target):
        cargo_target = self.root / 'target'
        executable = cargo_target / target / 'release' / ('neoko-terminal.exe' if system == 'Windows' else 'neoko-terminal')
        executable.parent.mkdir(parents=True)
        executable.write_bytes(self.payload)
        destination = self.root / 'dist'

        def run(*args, **kwargs):
            if args[0] == 'cargo':
                return
            if args[0] == 'codesign':
                app = Path(args[-1])
                self.signed_apps.append({path.relative_to(app).as_posix(): path.read_bytes()
                                         for path in app.rglob('*') if path.is_file()})
                return
            if args[0] == 'lipo':
                return
            if args[0] == 'ditto':
                app, archive_path = Path(args[-2]), Path(args[-1])
                with zipfile.ZipFile(archive_path, 'w', zipfile.ZIP_DEFLATED) as archive:
                    for path in app.rglob('*'):
                        if path.is_file():
                            archive.write(path, path.relative_to(app.parent))
                return
            return subprocess.run([str(arg) for arg in args], check=True, capture_output=True, **kwargs)

        with patch.object(builder, 'CLIENT', self.client), \
             patch.object(builder.platform, 'system', return_value=system), \
             patch.object(builder, 'run', side_effect=run), \
             patch.object(builder, 'output', side_effect=['glibc 2.39', '4.14.0', '2.80.0', '1.24.0']), \
             patch.dict(os.environ, {'CARGO_TARGET_DIR': str(cargo_target)}), \
             patch.object(sys, 'argv', ['build-desktop.py', '--version', '0.4.1', '--target', target, '--out', str(destination)]):
            builder.main()
        suffix = 'deb' if system == 'Linux' else 'zip'
        package = destination / f'neoko-terminal-0.4.1-{target}.{suffix}'
        checksum = hashlib.sha256(package.read_bytes()).hexdigest()
        self.assertEqual(package.with_name(package.name + '.sha256').read_text(), f'{checksum}  {package.name}\n')
        return package

    @unittest.skipUnless(DPKG_DEB, 'dpkg-deb is required to inspect a real deb')
    def test_linux_deb_contains_localized_menu_names_and_search_keywords(self):
        with patch.dict(os.environ, {'DPKG_DEB': DPKG_DEB}):
            package = self.build('Linux', 'x86_64-unknown-linux-gnu')
        extracted = self.root / 'extracted'
        subprocess.run([DPKG_DEB, '--extract', str(package), str(extracted)], check=True)
        desktop = extracted / 'usr/share/applications/community.lazycat.neko.remoted.desktop'
        if shutil.which('desktop-file-validate'):
            subprocess.run(['desktop-file-validate', str(desktop)], check=True)
        parser = configparser.ConfigParser(interpolation=None)
        parser.optionxform = str
        parser.read(desktop, encoding='utf-8')
        entry = parser['Desktop Entry']
        self.assertEqual(entry['Name'], '小橘终端')
        for locale in ('en', 'en_US'):
            self.assertEqual(entry[f'Name[{locale}]'], 'neoko-terminal')
        for locale in ('zh', 'zh_CN', 'zh_TW', 'zh_HK', 'zh_SG', 'zh_MO'):
            self.assertEqual(entry[f'Name[{locale}]'], '小橘终端')
        self.assertIn('terminal;', entry['Keywords'])
        self.assertIn('neoko-terminal;', entry['Keywords'])
        self.assertIn('小橘终端;', entry['Keywords'])
        self.assertIn('终端;', entry['Keywords'])
        self.assertEqual(entry['Exec'], 'neoko-terminal')
        self.assertEqual(entry['Icon'], 'community.lazycat.neko.remoted')
        self.assertEqual(entry['StartupWMClass'], 'community.lazycat.neko.remoted')
        self.assertEqual((extracted / 'usr/bin/neoko-terminal').read_bytes(), self.payload)
        for name, source in builder.native_fonts.LICENSE_SOURCES.items():
            self.assertEqual((extracted / 'usr/share/doc/neoko-terminal/licenses/fonts' / name).read_bytes(),
                             (builder.native_fonts.FONT_ROOT / source).read_bytes())
        self.assertEqual(subprocess.check_output([DPKG_DEB, '--field', str(package), 'Package'], text=True).strip(), 'neoko-terminal')
        if sys.platform == 'linux' and ctypes.util.find_library('gio-2.0'):
            probe = '''
import ctypes
import sys
library = ctypes.CDLL(sys.argv[1])
library.g_desktop_app_info_new_from_filename.argtypes = [ctypes.c_char_p]
library.g_desktop_app_info_new_from_filename.restype = ctypes.c_void_p
library.g_app_info_get_display_name.argtypes = [ctypes.c_void_p]
library.g_app_info_get_display_name.restype = ctypes.c_char_p
application = library.g_desktop_app_info_new_from_filename(sys.argv[2].encode())
if not application:
    raise RuntimeError('GIO could not load the packaged desktop entry')
print(library.g_app_info_get_display_name(application).decode())
'''
            for locale, name in (('C.UTF-8', '小橘终端'), ('en_US.UTF-8', 'neoko-terminal'),
                                 ('en_GB.UTF-8', 'neoko-terminal'), ('zh_CN.UTF-8', '小橘终端'),
                                 ('zh_TW.UTF-8', '小橘终端'), ('zh_HK.UTF-8', '小橘终端')):
                with self.subTest(locale=locale):
                    environment = {**os.environ, 'LANG': locale, 'LC_ALL': locale, 'LANGUAGE': '',
                                   'PATH': str(extracted / 'usr/bin') + os.pathsep + os.environ.get('PATH', '')}
                    name_from_gio = subprocess.check_output(
                        [sys.executable, '-c', probe, ctypes.util.find_library('gio-2.0'), str(desktop)],
                        env=environment, text=True).strip()
                    self.assertEqual(name_from_gio, name)

    def test_macos_zip_contains_localized_bundle_names_before_signing(self):
        for target in ('aarch64-apple-darwin', 'x86_64-apple-darwin'):
            with self.subTest(target=target):
                package = self.build('Darwin', target)
                prefix = 'neoko-terminal.app/Contents/'
                with zipfile.ZipFile(package) as archive:
                    for name, source in builder.native_fonts.LICENSE_SOURCES.items():
                        self.assertEqual(archive.read(prefix + 'Resources/licenses/fonts/' + name),
                                         (builder.native_fonts.FONT_ROOT / source).read_bytes())
                    info = plistlib.loads(archive.read(prefix + 'Info.plist'))
                    self.assertEqual(info['CFBundleIdentifier'], 'community.lazycat.neko.remoted')
                    self.assertEqual(info['CFBundleExecutable'], 'neoko-terminal')
                    self.assertEqual(info['CFBundleDisplayName'], 'neoko-terminal')
                    self.assertEqual(info['CFBundleDevelopmentRegion'], 'en')
                    self.assertEqual(set(info['CFBundleLocalizations']), {'en', 'zh', 'zh-Hans', 'zh-Hant'})
                    for locale, name in (('en', 'neoko-terminal'), ('zh', '小橘终端'),
                                         ('zh-Hans', '小橘终端'), ('zh-Hant', '小橘终端')):
                        resource = f'Resources/{locale}.lproj/InfoPlist.strings'
                        data = archive.read(prefix + resource)
                        strings = data.decode('utf-8')
                        self.assertIn(f'"CFBundleName" = "{name}";', strings)
                        self.assertIn(f'"CFBundleDisplayName" = "{name}";', strings)
                        self.assertEqual(self.signed_apps[-2]['Contents/' + resource], data)
                        self.assertEqual(self.signed_apps[-1]['Contents/' + resource], data)

    def test_windows_portable_zip_preserves_executable_and_unsigned_policy(self):
        package = self.build('Windows', 'x86_64-pc-windows-msvc')
        with zipfile.ZipFile(package) as archive:
            for name, source in builder.native_fonts.LICENSE_SOURCES.items():
                self.assertEqual(archive.read('licenses/fonts/' + name),
                                 (builder.native_fonts.FONT_ROOT / source).read_bytes())
            self.assertEqual(archive.read('neoko-terminal.exe'), self.payload)
            info = json.loads(archive.read('build-info.json'))
            self.assertEqual(info['signing'], 'unsigned')
            self.assertEqual(info['cargo_binary_sha256'], hashlib.sha256(self.payload).hexdigest())


@unittest.skipUnless(AAPT2.is_file() and ANDROID_JAR.is_file(), 'Android SDK 35 and build-tools 35.0.1 are required')
class AndroidPackageNameTests(unittest.TestCase):
    def test_aapt2_compiled_apk_contains_english_and_chinese_launcher_labels(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            resources = root / 'resources.zip'
            apk = root / 'neko-remoted.apk'
            assets = root / 'assets'
            builder.native_fonts.stage_android(assets)
            subprocess.run([str(AAPT2), 'compile', '--dir', str(CLIENT / 'android/res'), '-o', str(resources)], check=True)
            subprocess.run([str(AAPT2), 'link', '-I', str(ANDROID_JAR), '--manifest',
                            str(CLIENT / 'android/AndroidManifest.xml'), str(resources),
                            '-A', str(assets), '-o', str(apk)], check=True)
            builder.native_fonts.verify_apk(apk)
            with zipfile.ZipFile(apk) as archive:
                self.assertIn('resources.arsc', archive.namelist())
                self.assertIn('AndroidManifest.xml', archive.namelist())
            badging = subprocess.check_output([str(AAPT2), 'dump', 'badging', str(apk)], text=True)
            self.assertIn("package: name='community.lazycat.neko.remoted'", badging)
            self.assertIn("application-label:'neoko-terminal'", badging)
            self.assertIn("application-label-zh:'小橘终端'", badging)
            self.assertIn("launchable-activity: name='community.lazycat.neko.remoted.RemotedActivity'", badging)
            manifest = subprocess.check_output([str(AAPT2), 'dump', 'xmltree', '--file', 'AndroidManifest.xml', str(apk)], text=True)
            self.assertRegex(manifest, r'android:label\([^\n]+\)=@0x[0-9a-f]+')

    def test_font_staging_repairs_prebuilt_asset_overrides_and_verification_detects_corruption(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            assets = root / 'assets'
            stale = assets / 'neko-remoted-client/fonts/MapleMono-NF-CN-Regular.ttf'
            stale.parent.mkdir(parents=True)
            stale.write_bytes(b'stale prebuilt asset')
            builder.native_fonts.stage_android(assets)
            self.assertEqual(stale.read_bytes(),
                             (builder.native_fonts.FONT_ROOT / 'preinstalled' / stale.name).read_bytes())
            stale.write_bytes(b'broken font')
            apk = root / 'corrupted.apk'
            with zipfile.ZipFile(apk, 'w') as archive:
                for path in assets.rglob('*'):
                    if path.is_file():
                        archive.write(path, 'assets/' + path.relative_to(assets).as_posix())
            with self.assertRaisesRegex(ValueError, 'MapleMono-NF-CN-Regular.ttf'):
                builder.native_fonts.verify_apk(apk)


@unittest.skipUnless(sys.platform == 'linux' and shutil.which('cc') and shutil.which('pkg-config'),
                     'Linux C compiler and Pango development packages are required')
class LinuxFontFallbackTests(unittest.TestCase):
    def test_registered_font_stack_shapes_latin_chinese_and_nerd_symbols_without_system_fonts(self):
        flags = subprocess.run(['pkg-config', '--cflags', '--libs', 'pangocairo', 'pangoft2', 'fontconfig'],
                               text=True, capture_output=True)
        if flags.returncode:
            self.skipTest('Pango/Fontconfig development packages are required')
        with tempfile.TemporaryDirectory() as temporary:
            executable = Path(temporary) / 'font-fallback'
            subprocess.run(['cc', '-Wall', '-Wextra', '-Werror',
                            str(CLIENT / 'scripts/tests/font-fallback.c'), '-o', str(executable),
                            *flags.stdout.split()], check=True)
            source = (CLIENT / 'src/fonts.rs').read_text()
            family = re.search(r'const TERMINAL_FAMILY: &str\s*=\s*"([^"]+)";', source).group(1)
            result = subprocess.run([str(executable),
                                     *[str(builder.native_fonts.FONT_ROOT / 'preinstalled' / name)
                                       for name in builder.native_fonts.FONT_NAMES], family],
                                    text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn('小橘终端中文 => Maple Mono NF CN', result.stdout)
            self.assertIn('=> Symbols Nerd Font Mono', result.stdout)


if __name__ == '__main__':
    unittest.main()
