# synofurb brand

The mark is three chasing arrows forming a loop (the recycling-symbol convention: **re-furb-ish**) around a small drive-bay block with its status LED. The loop says "refurbish", the block says "NAS". It is an original drawing; the universal recycling symbol is a public-domain convention and nothing here is copied from a trademarked artwork.

Everything is generated: `python3 build.py` writes the SVGs, `python3 export.py` rasterises them with Chromium (Playwright) and assembles the PNG, ICO, ICNS, iconset and `packaging/icons/` files. Both need Python 3 with `fonttools` and `Pillow`, and Node with `playwright` (set `PW` and `CHROME` if they are not on the default path). The wordmark is **Inter SemiBold outlined to paths**, so no font is needed to display any SVG.

## Files

| File | Use |
|---|---|
| `synofurb-mark.svg` / `synofurb-mark-dark.svg` | Mark only, for light / dark backgrounds (violet arrows, ink or white bay, green LED) |
| `synofurb-logo.svg` / `synofurb-logo-dark.svg` | Mark + wordmark, "syno" in ink (white on dark) and "furb" in the accent violet |
| `synofurb-logo-mono.svg` | Mark + wordmark in `currentColor` (print, one-colour use, white on the violet app-icon colour) |
| `synofurb-mark-mono.svg`, `synofurb-mark-mono-small.svg` | One colour (`currentColor`), bay cut out; the **small** one has heavier strokes for 16 to 32 px: tray icon, favicon-like uses |
| `app-icon.svg`, `app-icon-small.svg`, `app-icon-macos.svg` | Rounded-square application icon (white mark on `#5E24BC`). Small = for 16 to 48 px; macOS = with the system margin |
| `png/synofurb-icon-{16,24,32,48,64,128,256,512,1024}.png` | App icon, full bleed (16 to 48 px use the small artwork) |
| `png/macos-*.png`, `synofurb.iconset/`, `synofurb.icns` | macOS artwork (with margin). `iconutil -c icns synofurb.iconset` rebuilds the `.icns` on a Mac; the committed `.icns` is already valid |
| `synofurb.ico` (16, 24, 32, 48, 64, 128, 256), `favicon.ico` (16, 32, 48), `favicon.svg` | Windows icon, favicon |
| `png/synofurb-mark-{256,512}.png`, `png/synofurb-mark-mono-*.png` | Transparent mark renders |
| `fonts/` | Inter (Regular, SemiBold, Bold) and JetBrains Mono (Regular, Bold) for bundling, with their OFL licence texts |
| `../../packaging/icons/` | The names cargo-packager expects (`32x32.png`, `128x128.png`, `128x128@2x.png`, `512x512.png`, `icon.png`, `icon.ico`, `icon.icns`), refreshed from these files |

## Colours

| Role | Light | Dark |
|---|---|---|
| Arrows, "furb" | `#7139CB` (6.67:1 on white) | `#A37BFE` (6.09:1 on `#121212`) |
| Bay block, "syno" | `#0A0A0A` (19.8:1 on white) | `#FFFFFF` / wordmark `#F4F1FF` (16.8:1 on `#121212`) |
| LED | `#04853A` | `#37BB62` |
| App icon background | `#5E24BC`; white on it is 8.56:1; LED `#7DF0A8` (6.07:1) | same |

The accent matches `design/v2/tokens-v2.json` (`accent`); do not use the logo's violet for UI states.

## Usage

- **Clear space:** at least 25% of the mark's height on every side (for the logo: the same, measured from the mark).
- **Minimum size:** mark 16 px (use the *small* artwork up to 48 px); logo 96 px wide. Below 96 px wide use the mark alone.
- **Backgrounds:** use the light files on white to light grey, the dark files on `#0A0A0A` to `#202020`. On the violet app colour or on photos use `synofurb-logo-mono.svg` in white. Do not put the violet "furb" on a violet or busy background (the contrast drops to about 1:1).
- **Do not** recolour the arrows, change their order or direction (clockwise), stretch or rotate the mark, add effects, or place the wordmark in another typeface. The wordmark may be Montserrat SemiBold only if the typeface is outlined the same way; Inter is the default.
- **Tray and favicons:** use `synofurb-mark-mono-small.svg` (`currentColor`) so the OS can tint it; macOS menu-bar icons should be supplied as template images (the mono file already is one).

## Licences

- **Artwork** (mark, logo, app icon): MIT, same as the project (`LICENSE`), with the request that modified versions are not presented as the official logo.
- **Inter** (wordmark outlines, UI font): SIL Open Font License 1.1, (c) 2016-2023 The Inter Project Authors. Outlining text into SVG paths is permitted by the OFL; the font files in `fonts/` may be bundled and redistributed with the application, with `fonts/OFL-Inter.txt`.
- **JetBrains Mono** (code, logs, hashes): SIL Open Font License 1.1, (c) 2020 The JetBrains Mono Project Authors (stated in the font files; `fonts/OFL-JetBrainsMono.txt`). Note the Debian package metadata for fonts-jetbrains-mono says Apache-2.0; the font files and upstream say OFL 1.1, which is what applies here.
- **Lucide icons** (UI): ISC licence, (c) Lucide Icons and Contributors (`LICENSE-Lucide.txt`, shipped by `gpui-kit-assets`). The logo does not use Lucide artwork.
- Add all three licences to the application's About page (Help and About, brief v2) and to the installers' third-party notices.
