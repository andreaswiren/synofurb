#!/usr/bin/env python3
"""Regenerate the embedded assets of synofurb_mgmt.py (between the BEGIN/END GENERATED ASSETS markers).

    python3 tools/build-assets.py            # needs: fontTools + brotli (pip install fonttools brotli), the repo's assets/brand, a Lucide icon set

Sources (all with licences that allow embedding; notices are shown on the About page):
  * fonts   assets/brand/fonts/*.otf|ttf  (Inter, JetBrains Mono: SIL OFL 1.1), subset to Latin and converted to woff2
  * logos   assets/brand/synofurb-{mark,logo}[-dark].svg and favicon.svg
  * icons   Lucide (ISC) from the gpui-kit-assets crate (LUCIDE_DIR=.../assets/icons overrides the cargo registry lookup)
The output is deterministic, so the diff only changes when a source changes.
"""
from __future__ import annotations

import base64
import glob
import io
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(ROOT))
BRAND = os.path.join(REPO, "assets", "brand")
TARGET = os.path.join(ROOT, "synofurb_mgmt.py")
BEGIN, END = "# BEGIN GENERATED ASSETS (tools/build-assets.py)", "# END GENERATED ASSETS"
UNICODES = "U+0020-007E,U+00A0-00FF,U+2013,U+2014,U+2018,U+2019,U+201C,U+201D,U+2022,U+2026,U+2192,U+2212,U+00D7"
FONTS = (("Inter", 400, "Inter-Regular.otf", "kern,tnum"), ("Inter", 600, "Inter-SemiBold.otf", "kern,tnum"), ("Inter", 700, "Inter-Bold.otf", "kern,tnum"),
         ("JetBrains Mono", 400, "JetBrainsMono-Regular.ttf", "kern"))
ICONS = ("check x shield-check users circle-question-mark circle-dashed triangle-alert ban octagon-x info server hard-drive cpu memory-stick network power gauge layers "
         "refresh-cw eye eye-off copy download chevron-down arrow-right arrow-left plug terminal trash lock key-round clock thermometer activity plus sun moon database "
         "undo-2 life-buoy flag menu wrench package settings panel-top search external-link book-open file-text recycle circle-check circle-x scroll-text clipboard-copy bell container camera play square globe").split()
LOGOS = {"mark": "synofurb-mark.svg", "mark_dark": "synofurb-mark-dark.svg", "logo": "synofurb-logo.svg", "logo_dark": "synofurb-logo-dark.svg"}


def lucide_dir() -> str:
    d = os.environ.get("LUCIDE_DIR")
    if d:
        return d
    hits = sorted(glob.glob(os.path.expanduser("~/.cargo/registry/src/*/gpui-kit-assets-*/assets/icons")))
    if not hits:
        sys.exit("Lucide icons not found: set LUCIDE_DIR to a directory of Lucide *.svg files")
    return hits[-1]


def subset(path: str, features: str) -> str:
    from fontTools import subset as fts
    opts = fts.Options()
    opts.flavor = "woff2"
    opts.layout_features = features.split(",")
    opts.hinting = False
    opts.desubroutinize = True
    font = fts.load_font(path, opts)
    s = fts.Subsetter(opts)
    s.populate(unicodes=fts.parse_unicodes(UNICODES))
    s.subset(font)
    buf = io.BytesIO()
    fts.save_font(font, buf, opts)
    return base64.b64encode(buf.getvalue()).decode()


def svg_parts(text: str) -> tuple[str, str]:
    vb = re.search(r'viewBox="([^"]+)"', text).group(1)
    inner = re.sub(r"</?title>.*?</title>", "", re.search(r"<svg[^>]*>(.*)</svg>", text, re.S).group(1), flags=re.S).strip()
    return vb, re.sub(r"\s+", " ", inner)


def generate() -> str:
    out = [BEGIN, "# Do not edit by hand. Lucide icons: ISC licence; Inter and JetBrains Mono: SIL OFL 1.1 (texts in _LICENSES, shown on the About page).", ""]
    out.append("_FONT_FACES = (")
    for fam, w, fn, feat in FONTS:
        out.append("    (%r, %d, %r)," % (fam, w, subset(os.path.join(BRAND, "fonts", fn), feat)))
    out.append(")")
    d = lucide_dir()
    out.append("_LUCIDE = {")
    for n in ICONS:
        p = os.path.join(d, n + ".svg")
        if not os.path.isfile(p):
            sys.exit("not a Lucide icon: %s" % n)
        _, inner = svg_parts(open(p, encoding="utf-8").read())
        out.append("    %r: %r," % (n, inner))
    out.append("}")
    out.append("_BRAND = {")
    for k, fn in LOGOS.items():
        vb, inner = svg_parts(open(os.path.join(BRAND, fn), encoding="utf-8").read())
        out.append("    %r: (%r, %r)," % (k, vb, inner))
    out.append("}")
    out.append("_FAVICON_B64 = %r" % base64.b64encode(open(os.path.join(BRAND, "favicon.svg"), "rb").read()).decode())
    lic = {"Inter": open(os.path.join(BRAND, "fonts", "OFL-Inter.txt"), encoding="utf-8").read().strip(),
           "JetBrains Mono": open(os.path.join(BRAND, "fonts", "OFL-JetBrainsMono.txt"), encoding="utf-8").read().strip(),
           "Lucide": open(os.path.join(os.path.dirname(os.path.dirname(d)), "LICENSE-LUCIDE"), encoding="utf-8").read().split("---")[0].strip()}
    out.append("_LICENSES = {")
    for k, v in lic.items():
        out.append("    %r: %r," % (k, v))
    out.append("}")
    out.append(END)
    return "\n".join(out)


def main() -> int:
    src = open(TARGET, encoding="utf-8").read()
    block = generate()
    if BEGIN in src:
        a, b = src.index(BEGIN), src.index(END) + len(END)
        src = src[:a] + block + src[b:]
    else:                                                       # first run: replace the hand-made sprite literal
        a = src.index("_ICON_SPRITE = '''")
        b = src.index("'''", a + 20) + 3
        src = src[:a] + block + src[b:]
    open(TARGET, "w", encoding="utf-8").write(src)
    print("wrote %d bytes of assets into %s" % (len(block), TARGET))
    return 0


if __name__ == "__main__":
    sys.exit(main())
