"""Generate the proposed macOS template icon and an optional design preview.

SVG is the source of truth. PNGs contain black RGB plus antialiased alpha only;
macOS, not the asset, supplies the final black/white tint when marked as template.
Requires Pillow at development time, never at application runtime.
"""
from __future__ import annotations

import argparse
import importlib.util
import sys
from pathlib import Path
import xml.etree.ElementTree as ET

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
SVG = ROOT / "assets/menubar-icon.svg"
# Reuse the existing SVG path parser, not a second hand-maintained geometry copy.
SPEC = importlib.util.spec_from_file_location("app_icon_geometry", ROOT / "scripts/generate-app-icon.py")
assert SPEC and SPEC.loader
GEOMETRY = importlib.util.module_from_spec(SPEC)
PREVIOUS_BYTECODE_SETTING = sys.dont_write_bytecode
try:
    sys.dont_write_bytecode = True
    SPEC.loader.exec_module(GEOMETRY)
finally:
    sys.dont_write_bytecode = PREVIOUS_BYTECODE_SETTING


def render(size: int) -> Image.Image:
    svg = ET.parse(SVG).getroot()
    assert svg.attrib["viewBox"] == "0 0 22 22"
    factor = size * 8 / 22
    alpha = Image.new("L", (size * 8, size * 8), 0)
    draw = ImageDraw.Draw(alpha)
    for element in svg:
        tag = element.tag.rsplit("}", 1)[-1]
        if tag == "path":
            assert element.attrib["fill"] == "none"
            assert element.attrib["stroke"] == "#000000"
            points, _ = GEOMETRY.path_points(element.attrib["d"])
            pixels = [(x * factor, y * factor) for x, y in points]
            width = round(float(element.attrib["stroke-width"]) * factor)
            draw.line(pixels, fill=255, width=width)
            radius = width / 2
            for x, y in pixels:
                draw.ellipse((x - radius, y - radius, x + radius, y + radius), fill=255)
        elif tag == "circle":
            assert element.attrib["fill"] == "#000000"
            x, y, r = (float(element.attrib[name]) * factor for name in ("cx", "cy", "r"))
            draw.ellipse((x - r, y - r, x + r, y + r), fill=255)
        else:
            raise ValueError(f"Unsupported SVG element: {tag}")
    result = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    result.putalpha(alpha.resize((size, size), Image.Resampling.LANCZOS))
    return result


def tinted(icon: Image.Image, color: str) -> Image.Image:
    result = Image.new("RGBA", icon.size, color)
    result.putalpha(icon.getchannel("A"))
    return result


def font(size: int, bold: bool = False) -> ImageFont.FreeTypeFont:
    candidates = [
        Path("C:/Windows/Fonts/msyhbd.ttc" if bold else "C:/Windows/Fonts/msyh.ttc"),
        Path("/System/Library/Fonts/PingFang.ttc"),
        Path("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
    ]
    for path in candidates:
        if path.is_file():
            return ImageFont.truetype(str(path), size)
    raise RuntimeError("Preview requires a local font; PNG template generation does not")


def preview(output: Path) -> None:
    board = Image.new("RGBA", (1280, 820), "#f5f6f8")
    draw = ImageDraw.Draw(board)

    def text(x: int, y: int, value: str, size: int, fill: str = "#20252c", bold: bool = False) -> None:
        draw.text((x, y), value, font=font(size, bold), fill=fill)

    text(52, 38, "Image Feedback", 21, "#687180")
    text(52, 79, "菜单栏，回到单色。", 40, bold=True)
    text(54, 143, "保留图片与反馈的轮廓。去掉彩色底板，让系统决定明暗。", 21, "#66707c")

    def paste_icon(x: int, y: int, size: int, color: str) -> None:
        board.alpha_composite(tinted(render(size), color), (x, y))

    def menu_bar(x: int, y: int, dark: bool) -> None:
        background, tint = ("#272a30", "#ffffff") if dark else ("#e5e7eb", "#22252a")
        draw.rounded_rectangle((x, y, x + 560, y + 136), 16,
                               fill="#363c46" if dark else "#d3dce5")
        draw.rounded_rectangle((x, y, x + 560, y + 56), 16, fill=background)
        draw.rectangle((x, y + 28, x + 560, y + 56), fill=background)
        text(x + 20, y + 15, "Finder", 19, tint, True)
        # These are design-context glyphs, not captured Apple UI assets.
        paste_icon(x + 290, y + 10, 36, tint)
        # Wi-Fi arcs, status dot, battery, and time establish relative icon weight.
        for radius in (9, 15, 21):
            draw.arc((x + 359 - radius, y + 41 - radius,
                      x + 359 + radius, y + 41 + radius), 225, 315, fill=tint, width=2)
        draw.ellipse((x + 357, y + 37, x + 361, y + 41), fill=tint)
        draw.rounded_rectangle((x + 397, y + 19, x + 429, y + 35), 4, outline=tint, width=2)
        draw.rectangle((x + 431, y + 24, x + 434, y + 30), fill=tint)
        draw.rounded_rectangle((x + 401, y + 23, x + 424, y + 31), 1, fill=tint)
        text(x + 457, y + 14, "16:42", 21, tint)
        # Quiet wallpaper only: keep the real-scale menu-bar mark as the visual focus.
        draw.line((x + 24, y + 106, x + 536, y + 106), fill="#424954" if dark else "#c1ccd7", width=1)

    text(54, 210, "浅色菜单栏", 23, bold=True)
    text(666, 210, "深色菜单栏", 23, bold=True)
    menu_bar(52, 251, False)
    menu_bar(664, 251, True)
    text(54, 406, "黑色轮廓 · 18 pt / Retina 2×", 18, "#6b7380")
    text(666, 406, "白色轮廓 · 18 pt / Retina 2×", 18, "#6b7380")

    draw.line((52, 465, 1228, 465), fill="#dfe3e8", width=1)
    text(54, 500, "同一个图形，不是两套配色", 25, bold=True)
    draw.rounded_rectangle((54, 558, 206, 710), 22, fill="#ffffff")
    draw.rounded_rectangle((226, 558, 378, 710), 22, fill="#25292f")
    paste_icon(70, 574, 120, "#25292f")
    paste_icon(242, 574, 120, "#ffffff")
    text(422, 557, "透明背景 · 图片框 + 反馈气泡", 23, bold=True)
    text(422, 602, "SVG 矢量源稿，导出 18 px / 36 px PNG。", 20, "#68717d")
    text(422, 640, "接入时设为 Template Image，自动适配明暗与选中态。", 20, "#68717d")
    text(422, 678, "Dock / Applications 的彩色图标保持不变。", 20, "#68717d")
    text(54, 767, "设计效果预览，非 macOS 实机截图；左下为放大检查。", 17, "#838b96")
    output.parent.mkdir(parents=True, exist_ok=True)
    board.convert("RGB").save(output)


def validate(image: Image.Image, size: int) -> None:
    assert image.size == (size, size) and image.mode == "RGBA"
    assert all(channel.getextrema() == (0, 0) for channel in image.split()[:3])
    alpha = image.getchannel("A")
    assert alpha.getextrema() == (0, 255)
    assert alpha.getbbox() is not None
    assert all(image.getpixel(point)[3] == 0 for point in ((0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preview", type=Path, help="Optional design-board PNG output")
    args = parser.parse_args()
    for size, name in ((18, "menubar-icon.png"), (36, "menubar-icon@2x.png")):
        image = render(size)
        validate(image, size)
        path = ROOT / "assets" / name
        image.save(path, optimize=True)
        with Image.open(path) as decoded:
            validate(decoded.convert("RGBA"), size)
        print(f"Validated alpha-only template: {path} ({size} x {size})")
    if args.preview:
        preview(args.preview)
        print(f"Design preview (not a native screenshot): {args.preview.resolve()}")


if __name__ == "__main__":
    main()
