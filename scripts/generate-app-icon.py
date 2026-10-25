"""Rasterize our original app-icon.svg with Pillow; no screenshot or external renderer.

Default: app-icon.png is 512x512. Use --png-size 256 to generate a 256x256 PNG
at that same path. The ICO always includes 16/20/24/32/40/48/64/128/256 px.
Only assets/app-icon.png and assets/app-icon.ico are written. SVG is canonical.
The tiny SVG subset below supports our rect/circle and absolute M/L/H/V/Q/Z paths.
"""
from __future__ import annotations

import argparse
import io
import math
import re
import struct
import xml.etree.ElementTree as ET
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
SVG_PATH = ROOT / "assets" / "app-icon.svg"
PNG_PATH = ROOT / "assets" / "app-icon.png"
ICO_PATH = ROOT / "assets" / "app-icon.ico"
ICO_SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)
SUPERSAMPLE = 4


def path_points(data: str) -> tuple[list[tuple[float, float]], bool]:
    tokens = re.findall(r"[MLHVQZ]|[-+]?(?:\d*\.)?\d+", data)
    points: list[tuple[float, float]] = []
    current = (0.0, 0.0)
    closed = False
    at = 0

    def number() -> float:
        nonlocal at
        value = float(tokens[at])
        at += 1
        return value

    while at < len(tokens):
        command = tokens[at]
        at += 1
        if command in ("M", "L"):
            current = (number(), number())
            points.append(current)
        elif command == "H":
            current = (number(), current[1])
            points.append(current)
        elif command == "V":
            current = (current[0], number())
            points.append(current)
        elif command == "Q":
            control = (number(), number())
            end = (number(), number())
            start = current
            # Subpixel curve approximation at 2048px, downsampled for every output.
            for step in range(1, 65):
                t = step / 64
                u = 1 - t
                points.append((u*u*start[0] + 2*u*t*control[0] + t*t*end[0],
                               u*u*start[1] + 2*u*t*control[1] + t*t*end[1]))
            current = end
        elif command == "Z":
            if points:
                points.append(points[0])
            closed = True
        else:
            raise ValueError(f"Unsupported SVG path command: {command}")
    return points, closed


def render_master() -> Image.Image:
    svg = ET.parse(SVG_PATH).getroot()
    if svg.attrib.get("viewBox") != "0 0 512 512":
        raise ValueError("Expected the canonical 512px SVG viewBox")
    image = Image.new("RGBA", (512 * SUPERSAMPLE, 512 * SUPERSAMPLE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    factor = SUPERSAMPLE

    def n(element: ET.Element, name: str) -> float:
        value = float(element.attrib[name])
        if not math.isfinite(value):
            raise ValueError("Non-finite SVG coordinate")
        return value

    for element in svg:
        tag = element.tag.rsplit("}", 1)[-1]
        if tag == "rect":
            x, y, width, height, radius = (n(element, key) for key in ("x", "y", "width", "height", "rx"))
            draw.rounded_rectangle((x*factor, y*factor, (x+width)*factor, (y+height)*factor),
                                   radius=radius*factor, fill=element.attrib["fill"])
        elif tag == "circle":
            x, y, radius = (n(element, key) for key in ("cx", "cy", "r"))
            draw.ellipse(((x-radius)*factor, (y-radius)*factor,
                          (x+radius)*factor, (y+radius)*factor), fill=element.attrib["fill"])
        elif tag == "path":
            if element.attrib.get("fill") != "none":
                raise ValueError("Only unfilled stroked paths are used by this original mark")
            points, closed = path_points(element.attrib["d"])
            pixels = [(x*factor, y*factor) for x, y in points]
            width = round(n(element, "stroke-width")*factor)
            color = element.attrib["stroke"]
            draw.line(pixels, fill=color, width=width)
            # Union round joints explicitly. Pillow's joint='curve' can leave tiny
            # raster cracks on densely sampled Q segments, especially after reduction.
            radius = width / 2
            if element.attrib.get("stroke-linejoin") == "round":
                for x, y in pixels:
                    draw.ellipse((x-radius, y-radius, x+radius, y+radius), fill=color)
            elif not closed and element.attrib.get("stroke-linecap") == "round":
                for x, y in (pixels[0], pixels[-1]):
                    draw.ellipse((x-radius, y-radius, x+radius, y+radius), fill=color)
        else:
            raise ValueError(f"Unsupported SVG element: {tag}")
    return image


def check_ico(path: Path) -> list[tuple[int, int]]:
    data = path.read_bytes()
    reserved, kind, count = struct.unpack_from("<HHH", data)
    if (reserved, kind, count) != (0, 1, len(ICO_SIZES)):
        raise ValueError("Unexpected ICO directory header")
    dimensions = []
    for index in range(count):
        width, height, _, _, planes, bits, length, offset = struct.unpack_from("<BBBBHHII", data, 6 + 16*index)
        width, height = width or 256, height or 256
        with Image.open(io.BytesIO(data[offset:offset+length])) as entry:
            if entry.size != (width, height):
                raise ValueError("ICO entry size disagrees with embedded PNG")
            if entry.mode != "RGBA":
                raise ValueError("ICO entry lost transparency")
        dimensions.append((width, height))
    if sorted(dimensions) != [(size, size) for size in ICO_SIZES]:
        raise ValueError(f"ICO missing requested sizes: {dimensions}")
    return dimensions


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--png-size", type=int, choices=(256, 512), default=512)
    args = parser.parse_args()
    master = render_master()
    # Verify both requested PNG resolutions without creating an unapproved extra file.
    for size in (256, 512):
        image = master.resize((size, size), Image.Resampling.LANCZOS)
        memory = io.BytesIO()
        image.save(memory, format="PNG")
        with Image.open(io.BytesIO(memory.getvalue())) as decoded:
            assert decoded.size == (size, size) and decoded.mode == "RGBA"
    master.resize((args.png_size, args.png_size), Image.Resampling.LANCZOS).save(PNG_PATH, optimize=True)
    base = master.resize((512, 512), Image.Resampling.LANCZOS)
    base.save(ICO_PATH, format="ICO", sizes=[(size, size) for size in ICO_SIZES], bitmap_format="png")
    sizes = check_ico(ICO_PATH)
    with Image.open(PNG_PATH) as png:
        assert png.size == (args.png_size, args.png_size) and png.mode == "RGBA"
        assert png.getpixel((0, 0))[3] == 0
    print(f"SVG source: {SVG_PATH} (512 x 512 viewBox)")
    print(f"PNG: {PNG_PATH} ({args.png_size} x {args.png_size}, RGBA)")
    print(f"ICO: {ICO_PATH} ({', '.join(str(w) for w, h in sizes)} px; all RGBA PNG entries)")
    print("Both 256px and 512px PNG encoding validated; geometry was read from the SVG.")


if __name__ == "__main__":
    main()
