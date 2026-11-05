"""Generates the app's images from assets/logo-source.webp.

Run after changing the source logo:

    pip install pillow numpy scipy
    python tools/make_icons.py

Writes assets/logo.png, assets/icon.png and assets/icon.ico.
"""
from pathlib import Path

import numpy as np
from PIL import Image, ImageChops, ImageDraw
from scipy import ndimage

ASSETS = Path(__file__).resolve().parent.parent / "assets"
TILE = (0x17, 0x17, 0x23, 255)   # The widget's title bar colour.
ICON_SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
BLACK_BELOW = 14                 # Brightness counted as background.
EDGE_PIXELS = 3                  # Width of the anti-aliased rim to recover.
SPECK_PIXELS = 64                # Compression noise left in the background.


def cut_out(source: Image.Image) -> Image.Image:
    """Removes the black around the logo, keeping black enclosed by it.

    Only black connected to the image border goes, so the thermometer's inner
    ring survives. Rim pixels are a blend of logo and black, so each gets the
    colour of the nearest solid pixel and an alpha from how bright it still is.
    That avoids the dark fringe a plain threshold leaves.
    """
    rgb = np.asarray(source.convert("RGB"), dtype=np.float32)
    brightness = rgb.max(axis=2)

    labels, _ = ndimage.label(brightness < BLACK_BELOW)
    border = np.concatenate([labels[0], labels[-1], labels[:, 0], labels[:, -1]])
    outside = np.isin(labels, np.unique(border[border > 0]))

    islands, count = ndimage.label(~outside)
    sizes = ndimage.sum(np.ones_like(islands), islands, range(1, count + 1))
    specks = np.flatnonzero(sizes < SPECK_PIXELS) + 1
    outside |= np.isin(islands, specks)

    rim = ndimage.binary_dilation(outside, iterations=EDGE_PIXELS) & ~outside
    solid = ~(outside | rim)

    nearest = ndimage.distance_transform_edt(~solid, return_distances=False, return_indices=True)
    colour = rgb[nearest[0], nearest[1]]

    alpha = np.ones(brightness.shape, dtype=np.float32)
    alpha[outside] = 0
    alpha[rim] = np.clip(brightness[rim] / np.maximum(colour.max(axis=2)[rim], 1), 0, 1)

    pixels = np.dstack([colour, alpha * 255]).round().astype(np.uint8)
    cut = Image.fromarray(pixels, "RGBA")
    return cut.crop(cut.getchannel("A").getbbox())


def on_tile(logo: Image.Image, size: int) -> Image.Image:
    """The logo on a dark rounded tile, so the white curves survive light themes."""
    scale = 4
    big = size * scale
    tile = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    ImageDraw.Draw(tile).rounded_rectangle((0, 0, big - 1, big - 1), radius=big * 0.2, fill=TILE)

    # Small icons have no detail to spare, so the logo is zoomed in, letting the
    # tips of the eye run off the tile to keep the thermometer readable.
    fill = 1.3 if size <= 24 else 0.96 if size <= 32 else 0.88
    width = round(big * fill)
    height = round(width * logo.height / logo.width)
    scaled = logo.resize((width, height), Image.LANCZOS)
    layer = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    layer.paste(scaled, ((big - width) // 2, (big - height) // 2))
    layer.putalpha(ImageChops.multiply(layer.getchannel("A"), tile.getchannel("A")))
    tile.alpha_composite(layer)
    return tile.resize((size, size), Image.LANCZOS)


def main() -> None:
    logo = cut_out(Image.open(ASSETS / "logo-source.webp"))

    wide = logo.resize((480, round(480 * logo.height / logo.width)), Image.LANCZOS)
    wide.save(ASSETS / "logo.png", optimize=True)

    icons = [on_tile(logo, size) for size in ICON_SIZES]
    icons[-1].save(ASSETS / "icon.png", optimize=True)
    icons[-1].save(
        ASSETS / "icon.ico",
        format="ICO",
        sizes=[(size, size) for size in ICON_SIZES],
        append_images=icons[:-1],
    )
    print(f"logo {wide.size}, icon sizes {ICON_SIZES}")


if __name__ == "__main__":
    main()
