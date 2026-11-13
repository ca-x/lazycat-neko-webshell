#!/usr/bin/env python3
"""Regenerate desktop ICO/ICNS from the shared brand PNG; requires ImageMagick."""
from pathlib import Path
import struct
import subprocess

CLIENT = Path(__file__).resolve().parents[1]
SOURCE = CLIENT / 'android/res/drawable/neko.png'
DESTINATION = CLIENT / 'assets/icons'


def main():
    DESTINATION.mkdir(parents=True, exist_ok=True)
    sizes = (16, 24, 32, 48, 64, 128, 256, 512, 1024)
    frames = {size: subprocess.check_output([
        'magick', str(SOURCE), '-resize', f'{size}x{size}', '-strip', 'PNG32:-'
    ]) for size in sizes}
    windows = (16, 24, 32, 48, 64, 128, 256)
    offset = 6 + 16 * len(windows)
    entries = []
    for size in windows:
        entries.append(struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0,
                                   1, 32, len(frames[size]), offset))
        offset += len(frames[size])
    (DESTINATION / 'neoko-terminal.ico').write_bytes(
        struct.pack('<HHH', 0, 1, len(windows)) + b''.join(entries)
        + b''.join(frames[size] for size in windows))
    macos = ((b'icp4', 16), (b'icp5', 32), (b'icp6', 64), (b'ic07', 128),
             (b'ic08', 256), (b'ic09', 512), (b'ic10', 1024),
             (b'ic11', 32), (b'ic12', 64), (b'ic13', 256), (b'ic14', 512))
    chunks = b''.join(kind + struct.pack('>I', len(frames[size]) + 8) + frames[size]
                      for kind, size in macos)
    (DESTINATION / 'neoko-terminal.icns').write_bytes(b'icns' + struct.pack('>I', len(chunks) + 8) + chunks)


if __name__ == '__main__':
    main()
