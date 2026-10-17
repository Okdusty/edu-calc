"""Regenerate checked-in GTK PNGs and the Windows ICO from qcalcp.svg.

Run with Python 3 and rsvg-convert on PATH. No Python packages are required.
"""
from pathlib import Path
import struct
import subprocess

root = Path(__file__).resolve().parent
sizes = (16, 24, 32, 48, 64, 128, 256)
images = []
for size in sizes:
    image = root / "icons" / f"{size}x{size}" / "apps" / "qcalcp.png"
    image.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([
        "rsvg-convert", "--width", str(size), "--height", str(size),
        "--output", str(image), str(root / "qcalcp.svg"),
    ], check=True)
    images.append(image.read_bytes())

# Windows Vista and newer support PNG images inside an ICO. Store every native
# size so Explorer and GTK don't have to enlarge a small taskbar bitmap.
offset = 6 + 16 * len(images)
directory = bytearray(struct.pack("<HHH", 0, 1, len(images)))
for size, image in zip(sizes, images):
    directory += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(image), offset)
    offset += len(image)
(root / "qcalcp.ico").write_bytes(directory + b"".join(images))
