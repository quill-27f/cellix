"""Draws the sample plots with nothing but the standard library."""
import math, struct, sys, zlib


def save(path, width, height, pixels):
    rows = b"".join(b"\0" + bytes(pixels[y * width * 3:(y + 1) * width * 3]) for y in range(height))

    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    with open(path, "wb") as out:
        out.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header))
        out.write(chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def plot(path, curves):
    width, height, margin = 900, 500, 50
    pixels = bytearray([255]) * (width * height * 3)

    def dot(x, y, colour, size=2):
        for dy in range(-size, size + 1):
            for dx in range(-size, size + 1):
                at = (int(y + dy) * width + int(x + dx)) * 3
                if 0 <= at < len(pixels) - 2:
                    pixels[at:at + 3] = bytes(colour)

    for x in range(margin, width - margin):
        dot(x, height - margin, (60, 60, 60), 0)
    for y in range(margin, height - margin):
        dot(margin, y, (60, 60, 60), 0)
    for function, colour in curves:
        for step in range(3000):
            t = step / 3000
            x = margin + t * (width - 2 * margin)
            y = height - margin - function(t) * (height - 2 * margin)
            dot(x, y, colour)
    save(path, width, height, pixels)


train = lambda t: 0.08 + 0.85 * math.exp(-5 * t)
valid = lambda t: 0.16 + 0.78 * math.exp(-4 * t) + 0.1 * t
plot(sys.argv[1], [(train, (37, 99, 235)), (valid, (220, 80, 40))])
