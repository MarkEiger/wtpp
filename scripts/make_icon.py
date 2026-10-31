"""Generate assets/wtpp.ico: a dark tile with a blue '>_' prompt (PNG-in-ICO)."""
import struct, sys, zlib

def png(size):
    bg, fg, accent = (32, 32, 32), (230, 230, 230), (59, 120, 255)
    s = size / 64
    def inside_rounded(x, y, r=10 * s):
        cx = min(max(x, r), size - 1 - r); cy = min(max(y, r), size - 1 - r)
        return (x - cx) ** 2 + (y - cy) ** 2 <= r * r
    def chevron(x, y):  # '>' from (14,20) to (30,32) to (14,44), stroke ~6
        u, v = x / s, y / s
        if not 12 <= u <= 32: return False
        t = (u - 14) * 12 / 16
        return abs(v - (20 + t)) <= 3.5 or abs(v - (44 - t)) <= 3.5 if u <= 30 else abs(v - 32) <= 3.5 and u <= 31
    rows = []
    for y in range(size):
        row = bytearray([0])
        for x in range(size):
            if not inside_rounded(x, y):
                row += bytes((0, 0, 0, 0)); continue
            u, v = x / s, y / s
            if chevron(x, y): c = accent
            elif 34 <= u <= 52 and 40 <= v <= 46: c = fg
            else: c = bg
            row += bytes((*c, 255))
        rows.append(bytes(row))
    raw = zlib.compress(b"".join(rows), 9)
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)) + chunk(b"IDAT", raw) + chunk(b"IEND", b"")

sizes = [16, 32, 48, 256]
images = [png(n) for n in sizes]
header = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
entries = b""
for n, img in zip(sizes, images):
    entries += struct.pack("<BBBBHHII", n % 256, n % 256, 0, 0, 1, 32, len(img), offset)
    offset += len(img)
open(sys.argv[1], "wb").write(header + entries + b"".join(images))
