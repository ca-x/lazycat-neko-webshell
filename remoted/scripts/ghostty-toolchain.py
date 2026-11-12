#!/usr/bin/env python3
"""Install the pinned native Ghostty toolchain; run builds without DOCS_RS."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
import urllib.error
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PINS_FILE = ROOT / 'ghostty/toolchain.json'


def pins():
    config = json.loads(PINS_FILE.read_text())
    dependency = tomllib.loads((ROOT / 'ghostty/Cargo.toml').read_text())['dependencies']['libghostty-vt']
    if dependency['rev'] != config['binding_revision'] or dependency.get('default-features') is not False:
        raise ValueError('Ghostty manifest differs from toolchain pin/static feature selection')
    locked = tomllib.loads((ROOT / 'client/Cargo.lock').read_text())['package']
    for name in ('libghostty-vt', 'libghostty-vt-sys'):
        entries = [item for item in locked if item['name'] == name]
        expected = f"git+https://github.com/uzaaft/libghostty-rs?rev={config['binding_revision']}#{config['binding_revision']}"
        if len(entries) != 1 or entries[0].get('source') != expected:
            raise ValueError(f'{name} lockfile revision differs from toolchain pin')
    return config


def host_platform(system=None, machine=None):
    system = system or platform.system()
    machine = (machine or platform.machine()).lower()
    cpu = {'amd64': 'x86_64', 'x86_64': 'x86_64', 'arm64': 'aarch64', 'aarch64': 'aarch64'}.get(machine)
    os_name = {'Linux': 'linux', 'Darwin': 'macos', 'Windows': 'windows'}.get(system)
    key = f'{cpu}-{os_name}'
    if key not in json.loads(PINS_FILE.read_text())['zig_archives']:
        raise ValueError(f'Unsupported toolchain host: {system}/{machine}')
    return key


def checksum(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def download(url, destination, expected):
    if destination.exists():
        if checksum(destination) != expected:
            raise ValueError(f'Cached archive SHA-256 mismatch: {destination}')
        return destination
    destination.parent.mkdir(parents=True, exist_ok=True)
    partial = destination.with_suffix(destination.suffix + '.partial')
    for attempt in range(3):
        try:
            request = urllib.request.Request(url, headers={'User-Agent': 'neoko-terminal-build'})
            started = time.monotonic()
            with urllib.request.urlopen(request, timeout=30) as response, partial.open('wb') as output:
                while chunk := response.read(1024 * 1024):
                    output.write(chunk)
                    if time.monotonic() - started > 300:
                        raise TimeoutError('Archive download exceeded five minutes')
            if checksum(partial) != expected:
                raise ValueError(f'Downloaded archive SHA-256 mismatch: {url}')
            partial.replace(destination)
            return destination
        except (OSError, urllib.error.URLError):
            if attempt == 2:
                raise
            time.sleep(2)
        finally:
            partial.unlink(missing_ok=True)


def unpack(archive, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent, prefix='extract-') as temporary:
        staging = Path(temporary)
        if archive.suffix == '.zip':
            with zipfile.ZipFile(archive) as source:
                for member in source.infolist():
                    if not (staging / member.filename).resolve().is_relative_to(staging.resolve()):
                        raise ValueError('Archive entry escapes extraction directory')
                source.extractall(staging)
        else:
            with tarfile.open(archive) as source:
                source.extractall(staging, filter='data')
        roots = list(staging.iterdir())
        if len(roots) != 1 or not roots[0].is_dir():
            raise ValueError('Expected one archive root directory')
        if destination.exists():
            shutil.rmtree(destination)
        shutil.move(roots[0], destination)


def executable(directory):
    return directory / 'zig' / ('zig.exe' if platform.system() == 'Windows' else 'zig')


def verify(directory, config):
    if json.loads((directory / 'pins.json').read_text()) != config:
        raise ValueError('Installed toolchain differs from pins; run setup again')
    if not (directory / 'source/build.zig').is_file():
        raise ValueError('Missing pinned Ghostty source')
    version = subprocess.check_output([str(executable(directory)), 'version'], text=True).strip()
    if version != config['zig_version']:
        raise ValueError(f'Zig version differs from pin: {version}')


def setup(directory, cache, config):
    host = host_platform()
    archive = config['zig_archives'][host]
    name = f"zig-{host}-{config['zig_version']}{archive['suffix']}"
    zig_archive = download(f"https://ziglang.org/download/{config['zig_version']}/{name}", cache / name, archive['sha256'])
    revision = config['ghostty_revision']
    source_archive = download(f'https://api.github.com/repos/ghostty-org/ghostty/tarball/{revision}',
                              cache / f'ghostty-{revision}.tar.gz', config['ghostty_archive_sha256'])
    unpack(zig_archive, directory / 'zig')
    unpack(source_archive, directory / 'source')
    (directory / 'pins.json').write_text(json.dumps(config, indent=2) + '\n')
    verify(directory, config)
    # Commands still use `run` below: GITHUB_ENV alone cannot unset DOCS_RS.
    if output := os.environ.get('GITHUB_ENV'):
        with open(output, 'a', encoding='utf-8') as stream:
            stream.write(f'NEKO_GHOSTTY_TOOLCHAIN={directory}\n')
    print(json.dumps({'host': host, 'toolchain': str(directory), **config}, indent=2))


def build_environment(directory, config, android=False):
    verify(directory, config)
    environment = dict(os.environ)
    environment.pop('DOCS_RS', None)
    environment.pop('GHOSTTY_ZIG_SYSTEM_DIR', None)
    environment.update(GHOSTTY_SOURCE_DIR=str(directory / 'source'), ZIG=str(executable(directory)),
                       NEKO_GHOSTTY_ZIG=str(executable(directory)), LIBGHOSTTY_VT_SYS_CPU='baseline')
    if android:
        if platform.system() != 'Linux':
            raise ValueError('Android packaging currently requires a Linux host')
        ndk = Path(environment['ANDROID_NDK_HOME'])
        properties = (ndk / 'source.properties').read_text()
        if f"Pkg.Revision = {config['android_ndk']}" not in properties:
            raise ValueError('Android NDK differs from pin')
        tool_bin = ndk / 'toolchains/llvm/prebuilt/linux-x86_64/bin'
        environment['ZIG'] = str(ROOT / 'ghostty/tools/zig-android.sh')
        environment['NEKO_GHOSTTY_ANDROID_API'] = str(config['android_api'])
        for target in ('aarch64', 'x86_64'):
            linker = tool_bin / f"{target}-linux-android{config['android_api']}-clang"
            if not linker.is_file():
                raise ValueError(f'Missing Android linker: {linker}')
            environment[f'CARGO_TARGET_{target.upper()}_LINUX_ANDROID_LINKER'] = str(linker)
    return environment


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest='operation', required=True)
    install = subparsers.add_parser('setup')
    install.add_argument('--directory', required=True, type=Path)
    install.add_argument('--cache', required=True, type=Path)
    run = subparsers.add_parser('run')
    run.add_argument('--directory', type=Path, default=os.environ.get('NEKO_GHOSTTY_TOOLCHAIN'))
    run.add_argument('--android', action='store_true')
    run.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    config = pins()
    if args.operation == 'setup':
        setup(args.directory.resolve(), args.cache.resolve(), config)
    else:
        if not args.directory:
            parser.error('run requires --directory or NEKO_GHOSTTY_TOOLCHAIN')
        command = args.command[1:] if args.command[:1] == ['--'] else args.command
        if not command:
            parser.error('run requires a command after --')
        return subprocess.call(command, env=build_environment(args.directory.resolve(), config, args.android))
    return 0


if __name__ == '__main__':
    sys.exit(main())
