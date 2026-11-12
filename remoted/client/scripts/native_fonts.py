#!/usr/bin/env python3
"""Stage WebShell's existing fonts and notices for native packages."""
import argparse
from pathlib import Path
import shutil
import zipfile

FONT_ROOT = Path(__file__).resolve().parents[3] / 'src/frontend/public/fonts'
FONT_NAMES = (
    'HackNerdFontMono-Regular.ttf',
    'SymbolsNerdFontMono-Regular.ttf',
    'MapleMono-NF-CN-Regular.ttf',
)
LICENSE_SOURCES = {
    'Hack-LICENSE.txt': 'licenses/Hack-LICENSE.txt',
    'NerdFontsSymbolsOnly-LICENSE.txt': 'licenses/NerdFontsSymbolsOnly-LICENSE.txt',
    'MapleMono-NF-CN-LICENSE.txt': 'preinstalled/MapleMono-NF-CN-LICENSE.txt',
}


def android_files():
    return {
        **{f'neko-remoted-client/fonts/{name}': FONT_ROOT / 'preinstalled' / name
           for name in FONT_NAMES},
        **{f'licenses/fonts/{name}': FONT_ROOT / source
           for name, source in LICENSE_SOURCES.items()},
    }


def stage_licenses(destination):
    destination = Path(destination) / 'fonts'
    destination.mkdir(parents=True, exist_ok=True)
    for name, source in LICENSE_SOURCES.items():
        shutil.copy2(FONT_ROOT / source, destination / name)


def stage_android(destination):
    # Also run after NEKO_ANDROID_ASSETS overrides: a prebuilt .so still needs
    # the exact registry assets. NSH base APKs use these same paths, not copies.
    for name, source in android_files().items():
        output = Path(destination) / name
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, output)


def verify_apk(path):
    with zipfile.ZipFile(path) as archive:
        for name, source in android_files().items():
            entry = 'assets/' + name
            if archive.namelist().count(entry) != 1 or archive.read(entry) != source.read_bytes():
                raise ValueError(f'Packaged terminal font or notice differs: {entry}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('stage-android', 'verify-apk'))
    parser.add_argument('path', type=Path)
    args = parser.parse_args()
    if args.operation == 'stage-android':
        stage_android(args.path)
    else:
        verify_apk(args.path)


if __name__ == '__main__':
    main()
