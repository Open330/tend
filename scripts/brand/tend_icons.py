#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow==11.0.0"]
# ///
"""Generate the Tend app icon for every platform from one geometry.

The four-room mark (assets/brand/tend/mark-rooms.svg, STATES.md) is placed on
a ground according to a *variant*. `apply` writes the variant into the repo;
`preview` renders every variant at every size and in place (Dock, Finder,
Launchpad, browser tab, Windows taskbar) into a directory, plus a
self-contained HTML comparison sheet.

    uv run scripts/brand/tend_icons.py apply B
    uv run scripts/brand/tend_icons.py preview --out /tmp/tend-icons
    uv run scripts/brand/tend_icons.py measure some-icon-1024.png ...

What `apply` writes (nothing else is touched):

    apps/tend-macos/Resources/AppIcon.icon/icon.json, Assets/Room*.svg
    apps/tend-ios/Resources/Assets.xcassets/AppIcon.appiconset/AppIcon-1024.png
    assets/icon.svg, assets/brand/tend/appicon-states.svg, appicon-states-1024.png
    crates/tend/src/dashboard/web/landing/mac-icon.webp

The bare mark (favicon `crates/tend/src/dashboard/web/icon.svg`, the macOS
menu bar, the Windows tray and window icon) has no ground and is the same in
every variant, so it is not written here.

`preview` uses Apple's own renderers when they are present (macOS with
Xcode): `actool` compiles each variant's Icon Composer bundle into a probe
app and `render-app-icon.swift` asks NSWorkspace to draw it, which is exactly
how Finder and the Dock draw Tend.app; `ictool` exports the dark, clear and
tinted styles. Elsewhere it falls back to a flat render.
"""

from __future__ import annotations

import argparse
import base64
import io
import json
import math
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

REPO = Path(__file__).resolve().parents[2]
CANVAS = 1024

# --- the mark ---------------------------------------------------------------

# Room outlines in the 64-unit design grid (the square spans 4..60, y down),
# the same as BrandMark.swift and crates/tend-app/src/mark.rs. Order is the
# paint order of today's assets.
ROOMS = {
    "Error": "M4 7 Q4 4 7 4 L20.734 4 L24.082 35.25 L4 35.25 Z",
    "Done": "M4 40.75 L24.672 40.75 L26.734 60 L7 60 Q4 60 4 57 Z",
    "Waiting": "M26.266 4 L57 4 Q60 4 60 7 L60 17.25 L27.685 17.25 Z",
    "Working": "M28.275 22.75 L60 22.75 L60 57 Q60 60 57 60 L32.266 60 Z",
}
ERROR, WAITING, DONE = "#FF5A3C", "#FFAA2B", "#3FBF84"
INK, PAPER = "#16181D", "#F2F3F5"
GRAPHITE_TOP, GRAPHITE_BOTTOM = "#2E323B", "#14161B"
# Icon Composer's automatic gradient for the graphite base, as exported into
# today's iOS PNG (sampled top and bottom rows).
GRAPHITE_AUTO = ((60, 65, 75), (27, 31, 38))
# The system's default (no fill) ground, sampled from macOS 27's rendering.
LIGHT_AUTO = ((255, 255, 255), (232, 233, 236))


def room_colors(working: str) -> dict[str, str]:
    return {"Error": ERROR, "Done": DONE, "Waiting": WAITING, "Working": working}


def flatten(d: str, steps: int = 12) -> list[tuple[float, float]]:
    """Polygon points of one room path (M/L/Q/Z only)."""
    tokens = d.replace("M", " M ").replace("L", " L ").replace("Q", " Q ").replace("Z", " Z ").split()
    pts: list[tuple[float, float]] = []
    i = 0
    while i < len(tokens):
        op = tokens[i]
        if op in ("M", "L"):
            pts.append((float(tokens[i + 1]), float(tokens[i + 2])))
            i += 3
        elif op == "Q":
            cx, cy, x, y = map(float, tokens[i + 1 : i + 5])
            x0, y0 = pts[-1]
            for s in range(1, steps + 1):
                t = s / steps
                pts.append(((1 - t) ** 2 * x0 + 2 * (1 - t) * t * cx + t * t * x,
                            (1 - t) ** 2 * y0 + 2 * (1 - t) * t * cy + t * t * y))
            i += 5
        else:
            i += 1
    return pts


# --- variants ---------------------------------------------------------------


@dataclass(frozen=True)
class Variant:
    key: str
    title: str
    summary: str
    margin: float  # gap between the canvas edge and the mark, as a fraction of the canvas
    ground: str  # "graphite" (today's) or "light" (the system's default fill)
    working: str  # colour of the working room

    @property
    def offset(self) -> float:
        return round(self.margin * CANVAS, 3)

    @property
    def scale(self) -> float:
        return round((1 - 2 * self.margin) * CANVAS / 56, 3)

    @property
    def span(self) -> float:
        return 1 - 2 * self.margin


VARIANTS = {
    v.key: v
    for v in [
        Variant("A", "Today", "Graphite plate, mark at 58% of the icon.", 0.21, "graphite", PAPER),
        Variant("B", "Grid", "Graphite plate, mark enlarged to 76%, about the median of the owner's Dock (77.5%).", 0.12, "graphite", PAPER),
        Variant("C", "Rooms only", "No plate of our own: the rooms fill the icon; the system's light fill shows in the passages.", 0.0, "light", INK),
        Variant("D", "Rooms, dark passages", "The rooms fill the icon; the passages stay graphite.", 0.0, "graphite", PAPER),
        Variant("E", "Light plate", "The light system fill as the plate, mark at 76%, working room in ink (the web favicon on a plate).", 0.12, "light", INK),
    ]
}


def fmt(x: float) -> str:
    s = f"{x:.3f}".rstrip("0").rstrip(".")
    return s if s else "0"


def transform(v: Variant, *, legacy: bool) -> str:
    # assets/icon.svg historically writes "-4.0 -4.0", the room layers "-4 -4".
    t = "-4.0 -4.0" if legacy else "-4 -4"
    return f"translate({fmt(v.offset)} {fmt(v.offset)}) scale({fmt(v.scale)}) translate({t})"


# --- SVG / Icon Composer outputs ---------------------------------------------


def room_layer_svg(v: Variant, room: str) -> str:
    color = room_colors(v.working)[room]
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">'
        f'<g transform="{transform(v, legacy=False)}"><path d="{ROOMS[room]}" fill="{color}"/></g></svg>\n'
    )


def icon_json(v: Variant) -> str:
    doc: dict = {}
    if v.ground == "graphite":
        doc["fill-specializations"] = [
            {"value": {"automatic-gradient": "extended-srgb:0.18039,0.19608,0.23137,1.00000"}},
            {"appearance": "dark", "value": "system-dark"},
        ]
    layers = []
    for room in ["Error", "Done", "Waiting", "Working"]:
        layer: dict = {"image-name": f"Room{room}.svg", "name": f"Room{room}"}
        if room == "Working" and v.working == INK:
            # Ink on the light fill; paper once the dark style darkens the fill.
            layer["fill-specializations"] = [
                {"appearance": "dark", "value": {"solid": "extended-srgb:0.94902,0.95294,0.96078,1.00000"}},
            ]
        layers.append(layer)
    doc["groups"] = [
        {
            "layers": layers,
            "name": "Rooms",
            "shadow": {"kind": "neutral", "opacity": 0.5},
            "translucency": {"enabled": False, "value": 0},
        }
    ]
    doc["supported-platforms"] = {"squares": ["macOS"]}
    return json.dumps(doc, indent=2)


def plate_svg(v: Variant) -> str:
    """assets/icon.svg: the flat, pre-rounded picture used outside Apple's renderers."""
    if v.ground == "graphite":
        stops = f'<stop offset="0" stop-color="{GRAPHITE_TOP}"/><stop offset="1" stop-color="{GRAPHITE_BOTTOM}"/>'
    else:
        stops = '<stop offset="0" stop-color="#FFFFFF"/><stop offset="1" stop-color="#E8E9EC"/>'
    rim = "#FFFFFF" if v.ground == "graphite" else "#000000"
    clip_open = '<clipPath id="c"><rect width="1024" height="1024" rx="230.4"/></clipPath>' if v.margin == 0 else ""
    paths = "".join(f'<path d="{ROOMS[r]}" fill="{c}"/>' for r, c in room_colors(v.working).items())
    group = f'<g transform="{transform(v, legacy=True)}">{paths}</g>'
    if v.margin == 0:
        group = f'<g clip-path="url(#c)">{group}</g>'
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1">'
        f"{stops}</linearGradient>{clip_open}</defs>"
        f'<rect x="0" y="0" width="1024" height="1024" rx="230.4" fill="url(#g)"/>'
        f'<rect x="1.5" y="1.5" width="1021" height="1021" rx="230.4" fill="none" stroke="{rim}" stroke-opacity=".06" stroke-width="3"/>'
        f"{group}</svg>"
    )


# --- flat raster renders ------------------------------------------------------


def hex_rgb(h: str) -> tuple[int, int, int]:
    return tuple(int(h[i : i + 2], 16) for i in (1, 3, 5))  # type: ignore[return-value]


def gradient(size: int, top, bottom) -> Image.Image:
    col = Image.new("RGB", (1, size))
    for y in range(size):
        t = y / max(1, size - 1)
        col.putpixel((0, y), tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(3)))
    return col.resize((size, size))


def draw_rooms(img: Image.Image, v: Variant, ss: int) -> None:
    d = ImageDraw.Draw(img)
    k = v.scale * ss
    o = v.offset * ss
    for room, color in room_colors(v.working).items():
        pts = [((x - 4) * k + o, (y - 4) * k + o) for x, y in flatten(ROOMS[room])]
        d.polygon(pts, fill=hex_rgb(color))


def render_square(v: Variant, size: int = CANVAS, ss: int = 4) -> Image.Image:
    """Opaque full-bleed square (the iOS App Store icon: the system masks it)."""
    top, bottom = GRAPHITE_AUTO if v.ground == "graphite" else LIGHT_AUTO
    big = gradient(size * ss, top, bottom)
    draw_rooms(big, v, ss * size / CANVAS)
    return big.resize((size, size), Image.LANCZOS)


def squircle_mask(size: int, inset: float = 0.0, n: float = 5.0, ss: int = 4) -> Image.Image:
    """Superellipse approximation of Apple's continuous-corner icon shape."""
    big = size * ss
    m = Image.new("L", (big, big), 0)
    px = m.load()
    lo, hi = inset * big, big - inset * big
    r = (hi - lo) / 2
    c = big / 2
    for y in range(big):
        dy = abs((y + 0.5 - c) / r)
        if dy >= 1:
            continue
        half = r * (1 - dy**n) ** (1 / n)
        for x in range(max(0, int(c - half)), min(big, int(c + half) + 1)):
            px[x, y] = 255
    return m.resize((size, size), Image.LANCZOS)


_MASKS: dict[tuple[int, float], Image.Image] = {}


def masked(img: Image.Image, inset: float = 0.0) -> Image.Image:
    size = img.width
    key = (size, inset)
    if key not in _MASKS:
        _MASKS[key] = squircle_mask(size, inset, ss=2 if size > 300 else 4)
    out = Image.new("RGBA", img.size, (0, 0, 0, 0))
    if inset:
        inner = round(size * (1 - 2 * inset))
        img = img.resize((inner, inner), Image.LANCZOS)
        canvas = Image.new("RGB", (size, size))
        canvas.paste(img, ((size - inner) // 2, (size - inner) // 2))
        img = canvas
    out.paste(img.convert("RGB"), (0, 0), _MASKS[key])
    return out


def bare_mark(size: int, working: str) -> Image.Image:
    """The mark with no ground (favicon, menu bar, tray), edge to edge."""
    v = Variant("bare", "", "", 0.0, "none", working)
    ss = 8
    big = Image.new("RGBA", (size * ss, size * ss), (0, 0, 0, 0))
    draw_rooms(big, v, ss * size / CANVAS)
    return big.resize((size, size), Image.LANCZOS)


def flat_mac(v: Variant, size: int) -> Image.Image:
    """macOS-style icon (squircle at 80.5% of the canvas) without Apple's renderer."""
    return masked(render_square(v, size), inset=(1 - 0.805) / 2)


# --- Apple's renderers (macOS + Xcode only) -----------------------------------

ICTOOL = Path("/Applications/Xcode.app/Contents/Applications/Icon Composer.app/Contents/Executables/ictool")


def have_apple_tools() -> bool:
    return sys.platform == "darwin" and shutil.which("xcrun") is not None and shutil.which("swiftc") is not None


def write_icon_bundle(v: Variant, dest: Path) -> None:
    (dest / "Assets").mkdir(parents=True, exist_ok=True)
    (dest / "icon.json").write_text(icon_json(v))
    for room in ROOMS:
        (dest / "Assets" / f"Room{room}.svg").write_text(room_layer_svg(v, room))


class MacRenderer:
    """Compiles an Icon Composer bundle into a throwaway app and lets NSWorkspace draw it."""

    def __init__(self, work: Path):
        self.work = work
        work.mkdir(parents=True, exist_ok=True)
        self.tool = work / "render-app-icon"
        subprocess.run(["swiftc", "-O", str(Path(__file__).with_name("render-app-icon.swift")), "-o", str(self.tool)], check=True)
        stub = work / "stub.c"
        stub.write_text("int main(void){return 0;}\n")
        self.exe = work / "stub"
        subprocess.run(["clang", "-O2", "-o", str(self.exe), str(stub)], check=True)
        self.nonce = os.urandom(4).hex()  # LaunchServices caches icons per bundle id and path

    def app(self, v: Variant, icon_dir: Path) -> Path:
        name = f"TendIcon{v.key}-{self.nonce}"
        app = self.work / f"{name}.app"
        out = self.work / f"car-{v.key}"
        shutil.rmtree(app, ignore_errors=True)
        shutil.rmtree(out, ignore_errors=True)
        out.mkdir()
        subprocess.run(
            ["xcrun", "actool", "--compile", str(out), "--app-icon", "AppIcon", "--platform", "macosx",
             "--minimum-deployment-target", "14.0", "--target-device", "mac",
             "--output-partial-info-plist", str(out / "partial.plist"), str(icon_dir)],
            check=True, capture_output=True,
        )
        (app / "Contents/MacOS").mkdir(parents=True)
        (app / "Contents/Resources").mkdir(parents=True)
        shutil.copy(self.exe, app / "Contents/MacOS/stub")
        for f in ("Assets.car", "AppIcon.icns"):
            shutil.copy(out / f, app / "Contents/Resources" / f)
        (app / "Contents/Info.plist").write_text(
            '<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" '
            '"http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict>'
            f"<key>CFBundleIdentifier</key><string>dev.tend.iconprobe.{v.key.lower()}{self.nonce}</string>"
            f"<key>CFBundleName</key><string>{name}</string><key>CFBundleExecutable</key><string>stub</string>"
            "<key>CFBundlePackageType</key><string>APPL</string><key>CFBundleIconFile</key><string>AppIcon</string>"
            "<key>CFBundleIconName</key><string>AppIcon</string></dict></plist>"
        )
        subprocess.run(["codesign", "-s", "-", "--force", str(app)], check=True, capture_output=True)
        return app

    def draw(self, app: Path, size: int) -> Image.Image:
        out = self.work / f"r-{app.stem}-{size}.png"
        subprocess.run([str(self.tool), str(out), str(size), str(app)], check=True)
        return Image.open(out).convert("RGBA")


def ictool(icon_dir: Path, out: Path, platform: str, rendition: str, size: int) -> Image.Image | None:
    if not ICTOOL.exists():
        return None
    args = [str(ICTOOL), str(icon_dir), "--export-image", "--output-file", str(out), "--platform", platform,
            "--rendition", rendition, "--width", str(size), "--height", str(size), "--scale", "1"]
    if rendition.startswith("Tinted"):
        args += ["--tint-color", "0.6", "--tint-strength", "0.75"]
    r = subprocess.run(args, capture_output=True)
    return Image.open(out).convert("RGBA") if r.returncode == 0 and out.exists() else None


# --- measuring ----------------------------------------------------------------


def measure(img: Image.Image, thr: int = 40) -> dict:
    """Plate and glyph extents of an icon: the glyph is whatever differs from the
    plate colour sampled just inside the plate's edge on the same row."""
    im = img.convert("RGBA")
    W, H = im.size
    px = im.load()
    l, t, r, b = im.getchannel("A").point(lambda a: 255 if a > 200 else 0).getbbox()
    pw, ph = r - l, b - t
    band = max(2, int(pw * 0.035))
    gx0, gy0, gx1, gy1 = W, H, 0, 0
    for y in range(t + band, b - band):
        xs = [x for x in range(l, r) if px[x, y][3] > 200]
        if len(xs) < 4 * band:
            continue
        x0, x1 = xs[0] + band, xs[-1] - band
        bl, br = px[x0, y], px[x1, y]
        for x in range(x0 + 1, x1):
            p = px[x, y]
            if min(max(abs(p[i] - bl[i]) for i in range(3)), max(abs(p[i] - br[i]) for i in range(3))) > thr:
                gx0, gy0, gx1, gy1 = min(gx0, x), min(gy0, y), max(gx1, x + 1), max(gy1, y + 1)
    if gx1 <= gx0:  # nothing on the plate differs: the artwork is full bleed
        gx0, gy0, gx1, gy1 = l, t, r, b
    return {
        "plate": round(pw / W, 3),
        "glyph": round(max(gx1 - gx0, gy1 - gy0) / pw, 3),
        "margin": round(min(gx0 - l, gy0 - t, r - gx1, b - gy1) / pw, 3),
    }


# --- apply --------------------------------------------------------------------


def apply(v: Variant, root: Path, mac_icon: Image.Image | None = None) -> list[Path]:
    written: list[Path] = []

    def put(rel: str, data: str | bytes) -> None:
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data.encode() if isinstance(data, str) else data)
        written.append(p)

    icon = "apps/tend-macos/Resources/AppIcon.icon"
    put(f"{icon}/icon.json", icon_json(v))
    for room in ROOMS:
        put(f"{icon}/Assets/Room{room}.svg", room_layer_svg(v, room))
    svg = plate_svg(v)
    put("assets/icon.svg", svg)
    put("assets/brand/tend/appicon-states.svg", svg)

    def png(img: Image.Image, fmt_: str = "PNG", **kw) -> bytes:
        buf = io.BytesIO()
        img.save(buf, fmt_, **kw)
        return buf.getvalue()

    square = render_square(v)
    put("assets/brand/tend/appicon-states-1024.png", png(masked(square), optimize=True))
    # App Store Connect rejects an icon with an alpha channel (#576).
    put("apps/tend-ios/Resources/Assets.xcassets/AppIcon.appiconset/AppIcon-1024.png", png(square.convert("RGB"), optimize=True))
    landing = mac_icon if mac_icon is not None else flat_mac(v, 256)
    put("crates/tend/src/dashboard/web/landing/mac-icon.webp", png(landing.resize((256, 256), Image.LANCZOS), "WEBP", quality=92, method=6))
    return written


# --- preview ------------------------------------------------------------------

SIZES = [16, 32, 64, 128, 512, 1024]
NEIGHBOURS = [  # the owner's Dock and the apps beside it (name, bundle path)
    ("Finder", "/System/Library/CoreServices/Finder.app"),
    ("Safari", "/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app"),
    ("Slack", "/Applications/Slack.app"),
    ("Claude", "/Applications/Claude.app"),
    ("ChatGPT", "/Applications/ChatGPT.app"),
    ("Ghostty", "/Applications/Ghostty.app"),
    ("Notion", "/Applications/Notion.app"),
    ("Linear", "/Applications/Linear.app"),
    ("Xcode", "/Applications/Xcode.app"),
    ("Settings", "/System/Applications/System Settings.app"),
    ("Chrome", "/Applications/Google Chrome.app"),
    ("OrbStack", "/Applications/OrbStack.app"),
    ("Crest", "/Applications/Crest.app"),
    ("Terminal", "/System/Applications/Utilities/Terminal.app"),
    ("Notes", "/System/Applications/Notes.app"),
]
WALLPAPERS = {"light": ((214, 225, 240), (160, 184, 214)), "dark": ((38, 44, 60), (12, 14, 22))}


def data_uri(img: Image.Image) -> str:
    buf = io.BytesIO()
    img.save(buf, "PNG", optimize=True)
    return "data:image/png;base64," + base64.b64encode(buf.getvalue()).decode()


def font(size: int):
    from PIL import ImageFont

    for f in ("/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"):
        if os.path.exists(f):
            return ImageFont.truetype(f, size)
    return ImageFont.load_default()


def wallpaper(w: int, h: int, mode: str) -> Image.Image:
    top, bottom = WALLPAPERS[mode]
    return gradient(max(w, h), top, bottom).resize((w, h)).convert("RGBA")


def dock(icons: list[Image.Image], mode: str, tile: int = 108) -> Image.Image:
    """A Dock at 54pt (@2x) over a wallpaper, icons as the system draws them."""
    pad = tile // 6
    w = len(icons) * tile + pad * 2
    h = tile + pad * 2
    bg = wallpaper(w + 80, h + 80, mode)
    shelf = Image.new("RGBA", bg.size, (0, 0, 0, 0))
    ImageDraw.Draw(shelf).rounded_rectangle((40, 40, 40 + w, 40 + h), radius=pad * 2,
                                            fill=(255, 255, 255, 90) if mode == "light" else (40, 40, 48, 150),
                                            outline=(255, 255, 255, 110 if mode == "light" else 40), width=2)
    bg.alpha_composite(shelf)
    for i, ic in enumerate(icons):
        bg.alpha_composite(ic.resize((tile, tile), Image.LANCZOS), (40 + pad + i * tile, 40 + pad))
    return bg


def finder_list(rows: list[tuple[str, Image.Image]], mode: str, px: int) -> Image.Image:
    """Finder list rows at 16pt (px=32 @2x) or 32pt."""
    rh = px + px // 2
    w, h = 420, rh * len(rows) + 16
    bg = Image.new("RGBA", (w, h), (255, 255, 255, 255) if mode == "light" else (30, 30, 32, 255))
    d = ImageDraw.Draw(bg)
    f = font(max(12, int(px * 0.55)))
    for i, (name, ic) in enumerate(rows):
        y = 8 + i * rh
        if i % 2:
            d.rectangle((0, y, w, y + rh), fill=(244, 245, 247, 255) if mode == "light" else (38, 38, 41, 255))
        bg.alpha_composite(ic.resize((px, px), Image.LANCZOS), (12, y + (rh - px) // 2))
        d.text((24 + px, y + rh // 2), name, font=f, anchor="lm", fill=(20, 20, 22) if mode == "light" else (235, 235, 240))
    return bg


def tab_strip(fav: Image.Image, mode: str) -> Image.Image:
    """A browser tab at 2x: the favicon is drawn at 16pt = 32px."""
    w, h = 520, 84
    bg = Image.new("RGBA", (w, h), (222, 225, 230, 255) if mode == "light" else (32, 33, 36, 255))
    d = ImageDraw.Draw(bg)
    d.rounded_rectangle((16, 14, 360, h + 20), radius=18, fill=(255, 255, 255, 255) if mode == "light" else (53, 54, 58, 255))
    bg.alpha_composite(fav.resize((32, 32), Image.LANCZOS), (40, 34))
    d.text((88, 50), "Home · tend", font=font(26), anchor="lm", fill=(30, 30, 30) if mode == "light" else (232, 234, 237))
    return bg


def taskbar(icon: Image.Image, mode: str) -> Image.Image:
    """Windows 11 taskbar at 150%: 48px buttons with 36px icons (24 logical px)."""
    w, h = 420, 72
    bg = Image.new("RGBA", (w, h), (238, 242, 247, 255) if mode == "light" else (28, 30, 34, 255))
    d = ImageDraw.Draw(bg)
    for i, c in enumerate([(0, 120, 212), (247, 196, 0), (110, 110, 110)]):
        d.rounded_rectangle((40 + i * 60, 18, 76 + i * 60, 54), radius=6, fill=c)
    x = 40 + 3 * 60
    d.rounded_rectangle((x - 6, 12, x + 42, 60), radius=8, fill=(255, 255, 255, 140) if mode == "light" else (60, 62, 68, 255))
    bg.alpha_composite(icon.resize((36, 36), Image.LANCZOS), (x, 18))
    d.rounded_rectangle((x + 10, 62, x + 26, 65), radius=2, fill=(0, 103, 192))
    return bg


def size_ladder(images: dict[int, Image.Image], mode: str) -> Image.Image:
    """Each size at its own pixel size, then the small ones blown up 4x (nearest)
    so the pixels that survive are visible."""
    shown = [16, 32, 64, 128]
    gap = 24
    zoom = [im for s in shown for im in [images[s].resize((s * 4, s * 4), Image.NEAREST)]]
    w = sum(s for s in shown) + sum(z.width for z in zoom) + gap * (len(shown) * 2 + 1)
    h = max(z.height for z in zoom) + gap * 2
    bg = wallpaper(w, h, mode)
    x = gap
    for s in shown:
        bg.alpha_composite(images[s], (x, gap + (h - 2 * gap - s) // 2))
        x += s + gap
    for z in zoom:
        bg.alpha_composite(z, (x, gap + (h - 2 * gap - z.height) // 2))
        x += z.width + gap
    return bg


def preview(out: Path) -> dict:
    out.mkdir(parents=True, exist_ok=True)
    apple = have_apple_tools()
    renderer = MacRenderer(out / "_work") if apple else None
    results: dict = {"apple_renderer": apple, "variants": {}}

    neighbours: list[tuple[str, Image.Image]] = []
    if renderer:
        for name, path in NEIGHBOURS:
            if os.path.exists(path):
                img = renderer.draw(Path(path), 256)
                neighbours.append((name, img))
                results.setdefault("neighbours", {})[name] = measure(renderer.draw(Path(path), 1024))

    for v in VARIANTS.values():
        vd = out / v.key
        tree = vd / "repo"
        icon_dir = tree / "apps/tend-macos/Resources/AppIcon.icon"
        write_icon_bundle(v, icon_dir)
        mac: dict[int, Image.Image] = {}
        if renderer:
            app = renderer.app(v, icon_dir)
            for s in SIZES:
                mac[s] = renderer.draw(app, s)
        else:
            for s in SIZES:
                mac[s] = flat_mac(v, s)
        apply(v, tree, mac_icon=mac[512])
        for s, im in mac.items():
            im.save(vd / f"macos-{s}.png")
        square = render_square(v)
        ios = masked(square.resize((360, 360), Image.LANCZOS))
        ios.save(vd / "ios-360.png")
        styles = {}
        if renderer:
            for r in ["Default", "Dark", "ClearLight", "ClearDark", "TintedLight", "TintedDark"]:
                im = ictool(icon_dir, vd / f"style-{r}.png", "macOS", r, 256)
                if im is not None:
                    styles[r] = im
        m = measure(mac[1024])
        results["variants"][v.key] = {"title": v.title, "summary": v.summary, "measure": m,
                                      "mark_span_of_canvas": round(v.span, 3)}
        lineup = [ic for _, ic in neighbours[:4]] + [mac[256] if 256 in mac else mac[512].resize((256, 256))] + [ic for _, ic in neighbours[4:8]]
        scenes = {
            "dock-light": dock(lineup, "light"),
            "dock-dark": dock(lineup, "dark"),
            "ladder-light": size_ladder(mac, "light"),
            "ladder-dark": size_ladder(mac, "dark"),
            "finder-light": finder_list([("Slack", neighbours[2][1] if len(neighbours) > 2 else mac[64]), ("Tend", mac[64]),
                                         ("Xcode", neighbours[8][1] if len(neighbours) > 8 else mac[64])], "light", 32),
            "finder-dark": finder_list([("Slack", neighbours[2][1] if len(neighbours) > 2 else mac[64]), ("Tend", mac[64]),
                                        ("Xcode", neighbours[8][1] if len(neighbours) > 8 else mac[64])], "dark", 32),
            "tab-light": tab_strip(mac[64], "light"),
            "tab-dark": tab_strip(mac[64], "dark"),
            "taskbar-light": taskbar(mac[64], "light"),
            "taskbar-dark": taskbar(mac[64], "dark"),
        }
        for name, im in scenes.items():
            im.save(vd / f"{name}.png")
        results["variants"][v.key]["files"] = sorted(p.name for p in vd.glob("*.png"))
        results["variants"][v.key]["_images"] = {"mac": mac, "ios": ios, "styles": styles, "scenes": scenes}

    # The bare mark shared by every variant: favicon, menu bar, Windows tray.
    results["_neighbours"] = neighbours
    summaries(out, results)
    write_sheet(out, results)
    clean = {k: v for k, v in results.items() if not k.startswith("_")}
    for vv in clean["variants"].values():
        vv.pop("_images", None)
    (out / "measurements.json").write_text(json.dumps(clean, indent=2))
    if renderer:
        shutil.rmtree(renderer.work, ignore_errors=True)
    return clean


def summaries(out: Path, results: dict) -> None:
    """Side-by-side composites of all variants, for docs/proposals/icon/."""
    keys = list(results["variants"])
    imgs = {k: results["variants"][k]["_images"] for k in keys}
    n = len(keys)
    # The 512px renders on a light and a dark ground.
    sheet = Image.new("RGBA", (256 * n, 512))
    for row, mode in enumerate(("light", "dark")):
        band = wallpaper(256 * n, 256, mode)
        for i, k in enumerate(keys):
            band.alpha_composite(imgs[k]["mac"][512].resize((256, 256), Image.LANCZOS), (i * 256, 0))
        sheet.paste(band, (0, row * 256))
    sheet.save(out / "summary-variants.png")
    # 16 and 32px, magnified 8x without smoothing.
    cell = 128 + 256 + 24
    small = Image.new("RGBA", (n * cell + 24, 560))
    for row, mode in enumerate(("light", "dark")):
        band = wallpaper(small.width, 280, mode)
        for i, k in enumerate(keys):
            x = 24 + i * cell
            band.alpha_composite(imgs[k]["mac"][16].resize((128, 128), Image.NEAREST), (x, 76))
            band.alpha_composite(imgs[k]["mac"][32].resize((256, 256), Image.NEAREST), (x + 136, 12))
        small.paste(band, (0, row * 280))
    small.save(out / "summary-small.png")
    # Icon styles from ictool, one row per variant.
    names = ["Default", "Dark", "ClearLight", "ClearDark", "TintedLight", "TintedDark"]
    if all(imgs[k]["styles"] for k in keys):
        st = Image.new("RGBA", (len(names) * 160, n * 160), (110, 120, 140, 255))
        for j, k in enumerate(keys):
            for i, r in enumerate(names):
                if r in imgs[k]["styles"]:
                    st.alpha_composite(imgs[k]["styles"][r].resize((150, 150), Image.LANCZOS), (i * 160 + 5, j * 160 + 5))
        st.save(out / "summary-styles.png")
    # The Dock, one strip per variant, light then dark.
    strips = [imgs[k]["scenes"]["dock-light"] for k in keys] + [imgs[k]["scenes"]["dock-dark"] for k in keys]
    dk = Image.new("RGBA", (strips[0].width * 2, strips[0].height * n))
    for i, im in enumerate(strips):
        dk.paste(im, ((i // n) * strips[0].width, (i % n) * strips[0].height))
    dk.save(out / "summary-dock.png")


def write_sheet(out: Path, results: dict) -> None:
    rows = []
    for key, r in results["variants"].items():
        im = r["_images"]
        m = r["measure"]
        sizes = "".join(
            f'<figure><img src="{data_uri(im["mac"][s])}" width="{min(s, 256) if s >= 64 else s}" height="{min(s, 256) if s >= 64 else s}">'
            f"<figcaption>{s}px</figcaption></figure>"
            for s in SIZES
        )
        styles = "".join(f'<figure><img src="{data_uri(i)}" width="96" height="96"><figcaption>{n}</figcaption></figure>'
                         for n, i in im["styles"].items())
        scenes = "".join(f'<figure class="scene"><img src="{data_uri(i)}" style="width:{i.width // 2}px"><figcaption>{n}</figcaption></figure>'
                         for n, i in im["scenes"].items())
        rows.append(
            f'<section id="v{key}"><h2>{key} · {r["title"]}</h2><p>{r["summary"]}</p>'
            f'<p class="m">Measured in the system rendering: plate {m["plate"]:.1%} of the canvas, '
            f'mark {m["glyph"]:.1%} of the plate, margin {m["margin"]:.1%} per side.</p>'
            f'<h3>Sizes (macOS rendering, 1px = 1px)</h3><div class="row">{sizes}</div>'
            f'<h3>iOS home screen shape</h3><div class="row"><img src="{data_uri(im["ios"])}" width="120" height="120"></div>'
            + (f'<h3>Icon styles (ictool)</h3><div class="row">{styles}</div>' if styles else "")
            + f'<h3>In place</h3><div class="row">{scenes}</div></section>'
        )
    ios_dir = out / "ios"
    ios_shots = "".join(
        f'<figure class="scene"><img src="{data_uri(Image.open(f))}" style="width:300px"><figcaption>{f.stem}</figcaption></figure>'
        for f in sorted(ios_dir.glob("ios-home-*.png")) if "crop" not in f.name
    ) if ios_dir.exists() else ""
    bare = "".join(
        f'<figure><div style="background:{bg};padding:10px;border-radius:8px"><img src="{data_uri(bare_mark(s, ink))}" width="{s}" height="{s}"></div>'
        f"<figcaption>{s}px on {name}</figcaption></figure>"
        for s in (16, 32) for name, bg, ink in (("light", "#fff", INK), ("dark", "#202124", PAPER))
    )
    nb = results.get("neighbours", {})
    table = "".join(f"<tr><td>{n}</td><td>{m['glyph']:.1%}</td><td>{m['margin']:.1%}</td></tr>" for n, m in sorted(nb.items(), key=lambda kv: kv[1]["glyph"]))
    html = f"""<title>Tend icon variants</title>
<style>
:root{{--bg:#f6f7f9;--fg:#16181d;--mute:#5f6368;--card:#fff;--line:#e3e5e8}}
@media (prefers-color-scheme:dark){{:root:not([data-theme=light]){{--bg:#111316;--fg:#f2f3f5;--mute:#9aa0a6;--card:#1b1d22;--line:#2a2d33;color-scheme:dark}}}}
:root[data-theme=dark]{{--bg:#111316;--fg:#f2f3f5;--mute:#9aa0a6;--card:#1b1d22;--line:#2a2d33;color-scheme:dark}}
body{{background:var(--bg);color:var(--fg);font:15px/1.5 -apple-system,system-ui,sans-serif;padding:24px 16px;max-width:1200px;margin:auto}}
section{{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:16px 20px;margin:20px 0}}
.row{{display:flex;flex-wrap:wrap;gap:16px;align-items:flex-end;overflow-x:auto}}
figure{{margin:0;text-align:center}} figcaption{{color:var(--mute);font-size:12px}}
.scene img{{max-width:100%;border-radius:8px}} .m{{color:var(--mute)}}
img{{image-rendering:auto}} table{{border-collapse:collapse}} td,th{{padding:2px 12px;border-bottom:1px solid var(--line);text-align:left}}
</style>
<h1>Tend icon variants</h1>
<p>Generated by <code>scripts/brand/tend_icons.py preview</code>. macOS images are drawn by macOS itself (actool + NSWorkspace) when available: {results['apple_renderer']}.</p>
<h2>Peer icons measured the same way</h2><table><tr><th>App</th><th>Glyph / plate</th><th>Margin</th></tr>{table}</table>
<section><h2>Shared by every variant: the bare mark</h2><p>The favicon, the macOS menu bar and the Windows tray and window icon draw the four rooms with no ground, edge to edge (100% of their box). They already are the owner's "square without a background" and stay as they are.</p><div class="row">{bare}</div></section>
{('<section><h2>iOS home screen (simulator, real install)</h2><p>Probe apps Tend A to E installed next to the real Tend (today = A).</p><div class=row>' + ios_shots + '</div></section>') if ios_shots else ''}
{''.join(rows)}
"""
    (out / "icon-variants.html").write_text(html)


# --- iOS simulator probe -------------------------------------------------------


def ios_probe(udid: str, out: Path) -> list[Path]:
    """Install one throwaway app per variant (bundle id dev.tend.iconprobe.*)
    into a simulator, screenshot the home screen in light and dark, then
    uninstall them. The simulator must already be booted."""
    out.mkdir(parents=True, exist_ok=True)
    work = out / "_ios"
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir()
    stub = work / "stub.c"
    stub.write_text("int main(void){return 0;}\n")
    exe = work / "stub"
    subprocess.run(["xcrun", "-sdk", "iphonesimulator", "clang", "-target", "arm64-apple-ios17.0-simulator",
                    "-o", str(exe), str(stub)], check=True)
    ids = []
    for v in VARIANTS.values():
        bundle_id = f"dev.tend.iconprobe.{v.key.lower()}"
        ids.append(bundle_id)
        app = work / f"Probe{v.key}.app"
        cat = work / f"Assets{v.key}.xcassets/AppIcon.appiconset"
        cat.mkdir(parents=True)
        render_square(v).convert("RGB").save(cat / "AppIcon-1024.png")
        (cat / "Contents.json").write_text(json.dumps({
            "images": [{"filename": "AppIcon-1024.png", "idiom": "universal", "platform": "ios", "size": "1024x1024"}],
            "info": {"author": "xcode", "version": 1}}))
        app.mkdir()
        subprocess.run(["xcrun", "actool", "--compile", str(app), "--app-icon", "AppIcon", "--platform", "iphonesimulator",
                        "--minimum-deployment-target", "17.0", "--target-device", "iphone",
                        "--output-partial-info-plist", str(work / f"partial{v.key}.plist"), str(cat.parent)],
                       check=True, capture_output=True)
        shutil.copy(exe, app / "stub")
        plist = {
            "CFBundleIdentifier": bundle_id, "CFBundleExecutable": "stub", "CFBundleName": f"Tend {v.key}",
            "CFBundleDisplayName": f"Tend {v.key}", "CFBundlePackageType": "APPL", "CFBundleVersion": "1",
            "CFBundleShortVersionString": "1.0", "MinimumOSVersion": "17.0", "CFBundleSupportedPlatforms": ["iPhoneSimulator"],
            "UIDeviceFamily": [1, 2], "UILaunchScreen": {},
            "CFBundleIcons": {"CFBundlePrimaryIcon": {"CFBundleIconName": "AppIcon", "CFBundleIconFiles": ["AppIcon60x60"]}},
            "CFBundleIcons~ipad": {"CFBundlePrimaryIcon": {"CFBundleIconName": "AppIcon", "CFBundleIconFiles": ["AppIcon60x60", "AppIcon76x76"]}},
        }
        import plistlib

        (app / "Info.plist").write_bytes(plistlib.dumps(plist))
        subprocess.run(["codesign", "-s", "-", "--force", str(app)], check=True, capture_output=True)
        subprocess.run(["xcrun", "simctl", "install", udid, str(app)], check=True)
    shots = []
    try:
        for mode in ("light", "dark"):
            subprocess.run(["xcrun", "simctl", "ui", udid, "appearance", mode], check=True)
            subprocess.run(["xcrun", "simctl", "launch", udid, "com.apple.springboard"], capture_output=True)
            subprocess.run(["sleep", "4"])
            shot = out / f"ios-home-{mode}.png"
            subprocess.run(["xcrun", "simctl", "io", udid, "screenshot", str(shot)], check=True, capture_output=True)
            shots.append(shot)
    finally:
        subprocess.run(["xcrun", "simctl", "ui", udid, "appearance", "light"], capture_output=True)
        for bundle_id in ids:
            subprocess.run(["xcrun", "simctl", "uninstall", udid, bundle_id], capture_output=True)
        shutil.rmtree(work, ignore_errors=True)
    return shots


# --- CLI ----------------------------------------------------------------------


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("apply", help="write a variant's icon assets into the repo")
    a.add_argument("variant", choices=sorted(VARIANTS))
    a.add_argument("--root", type=Path, default=REPO)
    p = sub.add_parser("preview", help="render every variant and the comparison sheet")
    p.add_argument("--out", type=Path, required=True)
    i = sub.add_parser("ios-probe", help="show every variant on a booted simulator's home screen")
    i.add_argument("udid")
    i.add_argument("--out", type=Path, required=True)
    m = sub.add_parser("measure", help="print plate and glyph extents of icon PNGs")
    m.add_argument("files", nargs="+", type=Path)
    args = ap.parse_args()
    if args.cmd == "apply":
        v = VARIANTS[args.variant]
        mac = None
        if have_apple_tools():
            import tempfile

            with tempfile.TemporaryDirectory() as tmp:
                r = MacRenderer(Path(tmp))
                icon_dir = Path(tmp) / "AppIcon.icon"
                write_icon_bundle(v, icon_dir)
                mac = r.draw(r.app(v, icon_dir), 512)
        for path in apply(v, args.root, mac_icon=mac):
            print(path.relative_to(args.root))
    elif args.cmd == "preview":
        res = preview(args.out)
        print(json.dumps(res, indent=2))
        print(f"sheet: {args.out / 'icon-variants.html'}")
    elif args.cmd == "ios-probe":
        for shot in ios_probe(args.udid, args.out):
            print(shot)
    else:
        for f in args.files:
            print(f, measure(Image.open(f)))


if __name__ == "__main__":
    main()
