#!/usr/bin/env python3
"""Write a 1024x1024 PNG: dark blue rounded square with a white 'N' (stdlib only)."""
import struct, sys, zlib

S = 1024
R = 200  # corner radius
BG = (0x1f, 0x33, 0x58)
FG = (0xf2, 0xf5, 0xfc)


def inside_round(x, y):
    cx = min(max(x, R), S - 1 - R)
    cy = min(max(y, R), S - 1 - R)
    return (x - cx) ** 2 + (y - cy) ** 2 <= R * R


def in_n(x, y):
    l, r, t, b, w = 290, 734, 270, 754, 84
    if not (t <= y <= b):
        return False
    if l <= x <= l + w or r - w <= x <= r:
        return True
    # diagonal from top-left to bottom-right
    u = (y - t) / (b - t)
    cx = l + w / 2 + u * (r - w / 2 - (l + w / 2))
    return abs(x - cx) <= w * 0.6


rows = []
for y in range(S):
    row = bytearray([0])
    for x in range(S):
        if not inside_round(x, y):
            row += bytes((0, 0, 0, 0))
        else:
            c = FG if in_n(x, y) else BG
            row += bytes((*c, 255))
    rows.append(bytes(row))


def chunk(tag, data):
    c = struct.pack(">I", len(data)) + tag + data
    return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)


png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", S, S, 8, 6, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(b"".join(rows), 9)) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
