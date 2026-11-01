#!/usr/bin/env python3
"""synofurb contrast + consistency gate (Direction 1, revised).

Usage:  python3 tools/contrast.py [tokens.json] [direction-1/styles.css] [-v]
Exit 1 on any failure.  It checks, for BOTH themes:
  1. MATRIX   every text token on every surface it is allowed to sit on (4.5:1), every non-text token
              (borders, focus ring, bars, dots) on every surface it can touch (3:1).
  2. CSS      every (color, background) and (border/outline, background) pair that actually occurs in the SAME
              rule of styles.css is extracted and tested (text 4.5, interactive boundary 3.0).
  3. COVERAGE every colour token referenced by styles.css must take part in at least one tested pair
              (decorative-only tokens are listed explicitly in DECORATIVE), so a new colour cannot slip in untested.
  4. ACCENTS  each accent offered on the web plane (tokens.json accent.<name>.*) is tested with its own label,
              link text and ring colours.
  5. LINT     no gradient / pattern / inset shadow / color-mix / backdrop-filter / uppercase transform, and no font
              size below 12 px, in styles.css and index.html (the design system says these are excluded or costly).
"""
import json, os, re, sys
HERE = os.path.dirname(os.path.abspath(__file__))
args = [a for a in sys.argv[1:] if not a.startswith('-')]
VERBOSE = '-v' in sys.argv
TOK = args[0] if args else os.path.join(HERE, '..', 'tokens.json')
CSS = args[1] if len(args) > 1 else os.path.join(HERE, '..', 'direction-1', 'styles.css')
T = json.load(open(TOK))
def lin(c): c /= 255; return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4
def lum(h): h = h.lstrip('#')[:6]; r, g, b = (int(h[i:i + 2], 16) for i in (0, 2, 4)); return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
def ratio(a, b): x, y = sorted((lum(a), lum(b)), reverse=True); return (x + 0.05) / (y + 0.05)

NEUTRAL = ['canvas', 'surface', 'surface_2', 'surface_3', 'sidebar', 'accent_bg']
STATUS = ['ok', 'warn', 'err', 'info']
TEXT = {  # foreground token -> surfaces it may sit on (4.5:1)
    'fg': NEUTRAL, 'fg_2': NEUTRAL, 'placeholder': ['canvas', 'surface', 'surface_2', 'accent_bg'],
    'accent_fg': NEUTRAL, 'on_accent': ['accent', 'accent_hover', 'accent_pressed'], 'on_danger': ['danger', 'danger_hover', 'danger_pressed'],
    'log_fg': ['log_bg', 'log_wrn_bg', 'log_err_bg'], 'log_dim': ['log_bg', 'log_wrn_bg', 'log_err_bg'], 'log_dbg': ['log_bg'],
    'log_inf': ['log_bg'], 'log_wrn': ['log_bg', 'log_wrn_bg'], 'log_err': ['log_bg', 'log_err_bg'],
}
for s in STATUS: TEXT[f'{s}_fg'] = [f'{s}_bg', 'canvas', 'surface', 'surface_2', 'surface_3', 'accent_bg']
SURF_ALL = ['canvas', 'surface', 'surface_2', 'surface_3', 'sidebar', 'accent_bg', 'ok_bg', 'warn_bg', 'err_bg', 'info_bg']
NONTEXT = {  # token -> surfaces it may touch (3:1)
    'border_strong': SURF_ALL, 'ring': SURF_ALL, 'ring_inverse': ['log_bg'], 'danger_edge': ['canvas', 'surface', 'sidebar'],
    'accent': ['canvas', 'surface', 'surface_2', 'surface_3'], 'on_accent': ['accent'],
}
for s in STATUS: NONTEXT[f'{s}_solid'] = ['canvas', 'surface', 'surface_2', 'surface_3', 'accent_bg']
EXTRA_NONTEXT = [('accent', 'accent_bg')]  # selected-row edge
DECORATIVE = {'border', 'scrim', 'accent_border', 'ok_border', 'warn_border', 'err_border', 'info_border', 'log_bg'}
fails = []; n = 0; covered = set()
def check(th, f, b, need, kind):
    global n
    fk, bk = f'color.{th}.{f}', f'color.{th}.{b}'
    if fk not in T or bk not in T: fails.append(f'{th} {kind}: unknown token {f} / {b}'); return
    r = ratio(T[fk], T[bk]); n += 1; covered.update((f, b))
    ok = r >= need
    if not ok: fails.append(f'{th:5} {kind:7} {f}({T[fk]}) on {b}({T[bk]}) = {r:.2f} < {need}')
    elif VERBOSE: print(f'{th:5} {kind:7} {f:14}{T[fk]} on {b:12}{T[bk]} {r:6.2f} >= {need} ok')
for th in ('light', 'dark'):
    for f, bs in TEXT.items():
        for b in bs: check(th, f, b, 4.5, 'text')
    for f, bs in NONTEXT.items():
        for b in bs: check(th, f, b, 3.0, 'nontext')
    for f, b in EXTRA_NONTEXT: check(th, f, b, 3.0, 'nontext')
n_matrix = n
# ---- accents
for name in sorted({k.split('.')[1] for k in T if k.startswith('accent.')}):
    for th in ('light', 'dark'):
        g = lambda k: T[f'accent.{name}.{th}.{k}']
        surf = T[f'color.{th}.surface']; canv = T[f'color.{th}.canvas']; s3 = T[f'color.{th}.surface_3']
        for lab, a, b, need in [('label', g('on_accent'), g('accent'), 4.5), ('label-hover', g('on_accent'), g('accent_hover'), 4.5), ('label-pressed', g('on_accent'), g('accent_pressed'), 4.5),
                                ('link on surface', g('accent_fg'), surf, 4.5), ('link on canvas', g('accent_fg'), canv, 4.5), ('link on tint', g('accent_fg'), g('accent_bg'), 4.5),
                                ('ring on surface', g('ring'), surf, 3), ('ring on tint', g('ring'), g('accent_bg'), 3), ('fill on surface', g('accent'), surf, 3), ('fill on track', g('accent'), s3, 3)]:
            r = ratio(a, b); n += 1
            if r < need: fails.append(f'{th:5} accent {name}: {lab} {a} on {b} = {r:.2f} < {need}')
            elif VERBOSE: print(f'{th:5} accent {name:7}{lab:16}{r:6.2f}')
n_acc = n - n_matrix
# ---- CSS extraction
n_css = 0; used = set(); css_text = ''
if os.path.exists(CSS):
    css_text = open(CSS).read()
    for m in re.finditer(r'([^{}]+)\{([^{}]*)\}', re.sub(r'/\*.*?\*/', '', css_text, flags=re.S)):
        sel, body = m.group(1).strip(), m.group(2)
        decl = dict()
        for d in body.split(';'):
            if ':' in d: k, v = d.split(':', 1); decl[k.strip()] = v.strip()
        tok = lambda v: [x.replace('-', '_') for x in re.findall(r'var\(--([\w-]+)\)', v or '')]
        for k, v in decl.items():
            if k in ('color', 'background', 'background-color', 'border-color', 'outline', 'outline-color', 'fill', 'stroke', 'border', 'border-top', 'border-bottom', 'border-left', 'box-shadow', 'caret-color', 'accent-color'): used.update(tok(v))
        fg = tok(decl.get('color')); bg = tok(decl.get('background') or decl.get('background-color'))
        bd = tok(decl.get('border-color') or decl.get('border') or decl.get('outline') or '')
        for th in ('light', 'dark'):
            if len(fg) == 1 and len(bg) == 1:
                check(th, fg[0], bg[0], 4.5, 'css-txt'); n_css += 1
            if len(bg) == 1:
                for x in bd:
                    if x in ('border_strong', 'ring'): check(th, x, bg[0], 3.0, 'css-bd'); n_css += 1
    tokens_defined = {k.split('.', 2)[2] for k in T if k.startswith('color.light.')}
    for u in sorted(used):
        if u not in tokens_defined and not re.match(r'^(e\d|t_.*|ts|r_.*|h(_.*)?|font|mono|ww|wh|vw|vh)$', u): fails.append(f'css: var(--{u.replace("_","-")}) is not a token in tokens.json')
        elif u in tokens_defined and u not in covered and u not in DECORATIVE: fails.append(f'coverage: token {u} is used in CSS colour properties but takes part in no tested pair')
# ---- lint
LINT = [(r'gradient', 'gradient'), (r'<pattern', 'SVG pattern'), (r'box-shadow:\s*inset', 'inset shadow'), (r'color-mix', 'color-mix'), (r'backdrop-filter', 'backdrop-filter'), (r'text-transform:\s*uppercase', 'uppercase transform')]
html = os.path.join(os.path.dirname(CSS), 'index.html')
for path in (CSS, html):
    if not os.path.exists(path): continue
    s = open(path).read()
    for rx, name in LINT:
        if re.search(rx, s): fails.append(f'lint: {name} found in {os.path.basename(path)}')
    for m in re.finditer(r'font-size:\s*([\d.]+)px', s):
        if float(m.group(1)) < 12: fails.append(f'lint: font-size {m.group(1)}px < 12px in {os.path.basename(path)}')
    for m in re.finditer(r'calc\(\s*([\d.]+)px\s*\*\s*var\(--ts', s):
        if float(m.group(1)) < 12: fails.append(f'lint: scaled font base {m.group(1)}px < 12px in {os.path.basename(path)}')
tot = n
print(f'Checked {tot} contrast pairs: {n_matrix} matrix (2 themes), {n_acc} accent-variant, {n_css} extracted from CSS rules.')
print(f'Passing {tot - len([f for f in fails if "lint" not in f and "coverage" not in f and "css:" not in f])} of {tot}; {len(fails)} failure(s) incl. coverage and lint.')
for f in fails: print('FAIL', f)
sys.exit(1 if fails else 0)
