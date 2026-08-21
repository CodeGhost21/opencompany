#!/usr/bin/env python3
"""Assemble frontend/public/favicon.ico from pre-rendered PNG frames.

Called by scripts/brand/generate-icons.sh, which renders each frame from the
vector at its final pixel size first. Doing it that way — rather than letting
an image library downscale one large raster — is what keeps the 16px frame
legible, since the mark's 16px strokes land on whole pixels only if they were
rasterised at 16px.

Written by hand rather than with Pillow because Pillow's ICO writer resizes
from a single base image and silently drops any requested size larger than it:
passing the 16px frame first yields a one-frame file, with no warning.

Usage: build_ico.py <dir-holding-ico-{16,32,48}.png> <output.ico>
"""

import struct
import sys

SIZES = (16, 32, 48)


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    work, out = sys.argv[1], sys.argv[2]

    frames = [open(f"{work}/ico-{size}.png", "rb").read() for size in SIZES]

    # ICONDIR, then one 16-byte ICONDIRENTRY per frame, then the payloads.
    # Frames are stored PNG-compressed, which every browser still in support
    # has been able to read since Vista.
    header = struct.pack("<HHH", 0, 1, len(frames))
    offset = len(header) + 16 * len(frames)

    entries = b""
    payload = b""
    for size, data in zip(SIZES, frames):
        # Width and height are single bytes, where 0 means 256; every size
        # here is well under that, so they are written literally.
        entries += struct.pack(
            "<BBBBHHII", size, size, 0, 0, 1, 32, len(data), offset
        )
        offset += len(data)
        payload += data

    with open(out, "wb") as fh:
        fh.write(header + entries + payload)


if __name__ == "__main__":
    main()
