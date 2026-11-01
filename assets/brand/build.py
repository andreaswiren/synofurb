#!/usr/bin/env python3
"""Generates the synofurb brand SVGs (mark, logo, dark logo, mono mark) and the app-icon source SVGs.
Text is outlined from Inter SemiBold (SIL OFL 1.1) with fontTools (pip install fonttools), so no font is needed to display the logo.
Run:  python3 assets/brand/build.py   then   node assets/brand/render.js   (PNG/ICO/iconset)."""
import math, os, sys
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
HERE = os.path.dirname(os.path.abspath(__file__))
VIOLET, VIOLET_DK, INK, WHITE, LED, LED_DK = '#7139CB', '#A37BFE', '#0A0A0A', '#FFFFFF', '#04853A', '#37BB62'
APP_BG = '#5E24BC'   # app icon background; white on it = 8.0:1

def arrows(cx, cy, R, w, head, rc, gap):
    """Three chasing arrows (recycle style) on a rounded triangle. Returns (stroke paths, head polygons)."""
    V = [(cx + R * math.cos(math.radians(-90 + 120 * i)), cy + R * math.sin(math.radians(-90 + 120 * i))) for i in range(3)]
    paths, heads = [], []
    for i in range(3):
        A, B, C = V[i], V[(i + 1) % 3], V[(i + 2) % 3]
        def lerp(P, Q, t): return (P[0] + (Q[0] - P[0]) * t, P[1] + (Q[1] - P[1]) * t)
        def unit(P, Q): d = math.hypot(Q[0] - P[0], Q[1] - P[1]); return ((Q[0] - P[0]) / d, (Q[1] - P[1]) / d)
        L = math.hypot(B[0] - A[0], B[1] - A[1])
        start = lerp(A, B, 0.5 + gap / (2 * L) * 0)  # midpoint of edge
        start = lerp(A, B, 0.56)
        u1 = unit(A, B); u2 = unit(B, C)
        p_in = (B[0] - u1[0] * rc, B[1] - u1[1] * rc)          # before corner
        p_out = (B[0] + u2[0] * rc, B[1] + u2[1] * rc)         # after corner
        end_t = 0.5 - gap / L                                   # head tip fraction along next edge
        tip = lerp(B, C, 0.46)
        base = (tip[0] - u2[0] * head, tip[1] - u2[1] * head)   # where the stroke ends and the head starts
        d = f'M{start[0]:.2f} {start[1]:.2f}L{p_in[0]:.2f} {p_in[1]:.2f}Q{B[0]:.2f} {B[1]:.2f} {p_out[0]:.2f} {p_out[1]:.2f}L{base[0]+u2[0]*1.0:.2f} {base[1]+u2[1]*1.0:.2f}'
        n = (-u2[1], u2[0]); hw = head * 0.74
        h = f'{tip[0]:.2f},{tip[1]:.2f} {base[0]+n[0]*hw:.2f},{base[1]+n[1]*hw:.2f} {base[0]-n[0]*hw:.2f},{base[1]-n[1]*hw:.2f}'
        paths.append(d); heads.append(h)
    return paths, heads

def bay(cx, cy, w, h, body, slot, led):
    """Drive-bay emblem: rounded block, a horizontal slot line cut out, an LED dot."""
    x, y = cx - w / 2, cy - h / 2
    s = f'<rect x="{x:.2f}" y="{y:.2f}" width="{w}" height="{h}" rx="{h*0.28:.2f}" fill="{body}"/>'
    s += f'<rect x="{x+w*0.16:.2f}" y="{cy-h*0.1:.2f}" width="{w*0.46:.2f}" height="{h*0.2:.2f}" rx="{h*0.1:.2f}" fill="{slot}"/>'
    s += f'<circle cx="{x+w*0.80:.2f}" cy="{cy:.2f}" r="{h*0.15:.2f}" fill="{led}"/>'
    return s

def mark_body(arrow_col, body_col, slot_col, led_col, sw=8.6, bg=None, small=False):
    cx, cy = 50, 55
    paths, heads = arrows(cx, cy, 40 if small else 38, sw, 17 if small else 14, 8, 0.0)
    g = ''.join(f'<path d="{d}" fill="none" stroke="{arrow_col}" stroke-width="{sw}" stroke-linecap="butt" stroke-linejoin="round"/><polygon points="{h}" fill="{arrow_col}" stroke="{arrow_col}" stroke-width="1.6" stroke-linejoin="round"/>' for d, h in zip(paths, heads))
    g += bay(cx, cy + 1, 22 if small else 23, 15 if small else 15.5, body_col, slot_col, led_col) if not small else f'<rect x="{cx-10}" y="{cy-5.5}" width="20" height="13" rx="4" fill="{body_col}"/><circle cx="{cx+5}" cy="{cy+1}" r="2.4" fill="{led_col}"/>'
    return g
def svg(vb, body, title, extra=''):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}" role="img" aria-label="{title}"{extra}><title>{title}</title>{body}</svg>\n'

# ---------- glyph outlines ----------
FONT = '/usr/share/fonts/opentype/inter/Inter-SemiBold.otf'
def text_paths(text, size, x0=0, y0=0, tracking=0.0):
    f = TTFont(FONT); gs = f.getGlyphSet(); cmap = f.getBestCmap(); upm = f['head'].unitsPerEm; k = size / upm
    out = []; x = x0
    for ch in text:
        gn = cmap[ord(ch)]; pen = SVGPathPen(gs)
        tp = TransformPen(pen, (k, 0, 0, -k, x, y0)); gs[gn].draw(tp); out.append((ch, pen.getCommands())); x += gs[gn].width * k + tracking * size
    return out, x - x0
def wordmark(size, x0, y0, col_a, col_b):
    parts, w = text_paths('synofurb', size, x0, y0, tracking=-0.012)
    s = ''
    for i, (ch, d) in enumerate(parts):
        s += f'<path d="{d}" fill="{col_b if i >= 4 else col_a}"/>'
    return s, w

MARK_LIGHT = mark_body(VIOLET, INK, WHITE, LED)
MARK_DARK = mark_body(VIOLET_DK, WHITE, '#17122B', LED_DK)
def logo(dark):
    wm, w = wordmark(62, 118, 74, '#F4F1FF' if dark else INK, VIOLET_DK if dark else VIOLET)
    body = f'<g>{MARK_DARK if dark else MARK_LIGHT}</g>' + wm
    return svg(f'0 0 {118 + w + 6:.0f} 108', body, 'synofurb')
def logo_mono():
    wm, w = wordmark(62, 118, 74, 'currentColor', 'currentColor')
    paths, heads = arrows(50, 55, 38, 9.4, 14, 8, 0.0)
    m = mono().split('<title>synofurb</title>')[1].rsplit('</svg>', 1)[0]
    return svg(f'0 0 {118 + w + 6:.0f} 108', f'<g>{m}</g>' + wm, 'synofurb')
def app_icon(margin, small=False):
    s = 1024; pad = s * margin
    inner = s - 2 * pad; r = inner * 0.2237
    k = inner * (0.90 if small else 0.78) / 100
    ox = pad + inner * (0.05 if small else 0.11); oy = pad + inner * (0.05 if small else 0.11) - inner * 0.01
    body = f'<rect x="{pad:.1f}" y="{pad:.1f}" width="{inner:.1f}" height="{inner:.1f}" rx="{r:.1f}" fill="{APP_BG}"/>'
    body += f'<g transform="translate({ox:.1f} {oy:.1f}) scale({k:.4f})">{mark_body(WHITE, WHITE, APP_BG, "#7DF0A8", sw=(13.5 if small else 9.2), small=small)}</g>'
    return svg('0 0 1024 1024', body, 'synofurb app icon')
def mono(small=False):
    paths, heads = arrows(50, 55, 40 if small else 38, 11 if small else 9.4, 17 if small else 14, 8, 0.0)
    g = ''.join(f'<path d="{d}" fill="none" stroke="currentColor" stroke-width="{11 if small else 9.4}" stroke-linejoin="round"/><polygon points="{h}" fill="currentColor" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/>' for d, h in zip(paths, heads))
    cx, cy, w, h = 50, 56, 23, 15.5
    x, y = cx - w / 2, cy - h / 2
    # bay as one path with the slot and LED cut out (evenodd)
    g += (f'<path fill="currentColor" fill-rule="evenodd" d="M{x+4.8:.2f} {y:.2f}H{x+w-4.8:.2f}A4.8 4.8 0 0 1 {x+w:.2f} {y+4.8:.2f}V{y+h-4.8:.2f}A4.8 4.8 0 0 1 {x+w-4.8:.2f} {y+h:.2f}H{x+4.8:.2f}A4.8 4.8 0 0 1 {x:.2f} {y+h-4.8:.2f}V{y+4.8:.2f}A4.8 4.8 0 0 1 {x+4.8:.2f} {y:.2f}Z'
          f'M{x+w*0.16:.2f} {cy-h*0.1:.2f}h{w*0.46:.2f}v{h*0.2:.2f}h-{w*0.46:.2f}ZM{x+w*0.80+h*0.15:.2f} {cy:.2f}a{h*0.15:.2f} {h*0.15:.2f} 0 1 0 -{h*0.30:.2f} 0a{h*0.15:.2f} {h*0.15:.2f} 0 1 0 {h*0.30:.2f} 0Z"/>')
    return svg('0 0 100 100', g, 'synofurb', ' color="#000"')
if __name__ == '__main__':
    W = lambda n, s: open(os.path.join(HERE, n), 'w').write(s)
    W('synofurb-mark.svg', svg('0 0 100 100', MARK_LIGHT, 'synofurb'))
    W('synofurb-mark-dark.svg', svg('0 0 100 100', MARK_DARK, 'synofurb'))
    W('synofurb-mark-mono.svg', mono())
    W('synofurb-logo.svg', logo(False)); W('synofurb-logo-dark.svg', logo(True))
    W('app-icon.svg', app_icon(0.0)); W('app-icon-macos.svg', app_icon(0.098))
    W('app-icon-small.svg', app_icon(0.0, True)); W('synofurb-mark-mono-small.svg', mono(True))
    W('synofurb-logo-mono.svg', logo_mono())
    print('ok')
