"""Shared native packaging inputs; additive metadata and pinned Ghostty notices."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE_PATHS = (
    'remoted/client', 'remoted/protocol', 'remoted/transport', 'remoted/host', 'remoted/themes',
    'remoted/ghostty', 'crates/hc-client', 'crates/program-status', 'remoted/scripts',
    'Cargo.toml', 'Cargo.lock', 'remoted/Cargo.toml', 'remoted/Cargo.lock',
    'scripts/export-native-terminal-themes.mjs',
    '.github/workflows/neoko-terminal-desktop.yml',
    '.github/workflows/neoko-terminal-android.yml',
)


def git(*args):
    return subprocess.check_output(['git', '-C', str(ROOT.parent), *args], text=True).strip()


def metadata():
    pins = json.loads((ROOT / 'ghostty/toolchain.json').read_text())
    return {
        'source_sha': git('rev-parse', 'HEAD'),
        'source_dirty': bool(git('status', '--porcelain', '--untracked-files=all', '--', *SOURCE_PATHS)),
        'ghostty_source_revision': pins['ghostty_revision'],
        'ghostty_wrapper_revision': pins['binding_revision'],
        'zig_version': pins['zig_version'],
    }


def license_files():
    source = ROOT / 'ghostty/licenses'
    record = json.loads((source / 'manifest.json').read_text())
    pins = json.loads((ROOT / 'ghostty/toolchain.json').read_text())
    if (record['ghostty_source_revision'] != pins['ghostty_revision'] or
            record['ghostty_wrapper_revision'] != pins['binding_revision']):
        raise ValueError('Ghostty license revisions differ from toolchain pins')
    for name, item in record['files'].items():
        path = source / name
        if not path.resolve().is_relative_to(source.resolve()):
            raise ValueError('License manifest path escapes source directory')
        if hashlib.sha256(path.read_bytes()).hexdigest() != item['sha256']:
            raise ValueError(f'Ghostty license digest differs: {name}')
    names = [*record['files'], 'manifest.json']
    return {name: (source / name).read_bytes() for name in names}


def stage_licenses(destination):
    # Validate before copying; an incomplete or stale attribution set fails packaging.
    for name, data in license_files().items():
        output = destination / 'ghostty' / name
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(data)


def verify_zip(path, prefix, expected_metadata=None):
    with zipfile.ZipFile(path) as archive:
        for name, expected in license_files().items():
            entry = str(PurePosixPath(prefix) / 'licenses/ghostty' / name)
            if archive.namelist().count(entry) != 1 or archive.read(entry) != expected:
                raise ValueError(f'Packaged Ghostty license differs: {entry}')
        if expected_metadata is not None:
            entry = f'{prefix}/build-inputs.json'
            if archive.namelist().count(entry) != 1 or archive.read(entry) != expected_metadata.read_bytes():
                raise ValueError('Packaged native build metadata differs')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='operation', required=True)
    emit = sub.add_parser('metadata')
    emit.add_argument('output', type=Path)
    stage = sub.add_parser('stage-licenses')
    stage.add_argument('destination', type=Path)
    verify = sub.add_parser('verify-apk')
    verify.add_argument('apk', type=Path)
    verify.add_argument('metadata', type=Path)
    merge = sub.add_parser('merge-metadata')
    merge.add_argument('build_info', type=Path)
    merge.add_argument('metadata', type=Path)
    args = parser.parse_args()
    if args.operation == 'metadata':
        args.output.write_text(json.dumps(metadata(), indent=2) + '\n')
    elif args.operation == 'stage-licenses':
        stage_licenses(args.destination)
    elif args.operation == 'verify-apk':
        verify_zip(args.apk, 'assets', args.metadata)
    else:
        info = json.loads(args.build_info.read_text())
        info.update(json.loads(args.metadata.read_text()))
        args.build_info.write_text(json.dumps(info, indent=2) + '\n')


if __name__ == '__main__':
    main()
