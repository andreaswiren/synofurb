#!/usr/bin/env python3
"""Builds index.html (self-contained: fonts base64, inline Lucide sprite, inline logo, help text from design/v2/UX-SPEC.md).
Usage: python3 design/mockups-v2/desktop/build.py   (needs fonttools only to refresh the font subsets under src/)"""
import base64, glob, json, os, re
H = os.path.dirname(os.path.abspath(__file__)); ROOT = os.path.abspath(os.path.join(H, '..', '..', '..'))
SRC = os.path.join(H, 'src')
T = json.load(open(os.path.join(ROOT, 'design', 'v2', 'tokens-v2.json')))
def rd(p, m='r'): return open(p, m).read()
# ---- tokens -> CSS
def vars_(th):
    out = []
    for k, v in T.items():
        if k.startswith(f'color.{th}.'): out.append(f'--{k.split(".",2)[2].replace("_","-")}:{v};')
    return ''.join(out)
extra = '--sb-thumb:var(--scrollbar-thumb);--sb-thumb-hover:var(--scrollbar-thumb-hover);--link:var(--accent-fg);'
tokens_css = f':root{{{vars_("light")}{extra}color-scheme:light}}:root[data-theme=dark]{{{vars_("dark")}{extra}color-scheme:dark}}'
# ---- fonts
def ff(name, weight, f, fmt='woff'): return f'@font-face{{font-family:"{name}";font-weight:{weight};font-style:normal;src:url(data:font/woff;base64,{base64.b64encode(open(os.path.join(SRC,f),"rb").read()).decode()}) format("woff")}}'
fonts_css = ''.join([ff('Inter', 400, 'inter-400.woff'), ff('Inter', 600, 'inter-600.woff'), ff('Inter', 700, 'inter-700.woff'), ff('JetBrains Mono', 400, 'jbm-400.woff'), ff('JetBrains Mono', 700, 'jbm-700.woff')])
# ---- JS
js = '\n'.join(rd(os.path.join(SRC, f)) for f in ('js1.js', 'js2.js', 'js3.js', 'js4.js', 'js5.js', 'js6.js', 'js7.js'))
# ---- icons (Lucide, ISC, from gpui-kit-assets)
icon_dir = glob.glob(os.path.expanduser('~/.cargo/registry/src/*/gpui-kit-assets-0.8.1/assets/icons'))[0]
have = {f[:-4] for f in os.listdir(icon_dir) if f.endswith('.svg')}
want = sorted({m for m in re.findall(r"['\"]([a-z][a-z0-9-]*)['\"]", js + rd(os.path.join(SRC, 'app.css'))) if m in have} | {'check', 'x', 'info', 'chevron-down', 'chevron-right', 'arrow-left', 'arrow-right'})
def inner(n): s = rd(os.path.join(icon_dir, n + '.svg')); return re.sub(r'<svg[^>]*>|</svg>', '', s, flags=re.S).strip()
sprite = '<svg width="0" height="0" style="position:absolute" aria-hidden="true">' + ''.join(f'<symbol id="i-{n}" viewBox="0 0 24 24">{inner(n)}</symbol>' for n in want) + '</svg>'
# ---- logo
B = os.path.join(ROOT, 'assets', 'brand')
def strip(s): return re.sub(r'<title>.*?</title>', '', s)
mk_l, mk_d = strip(rd(os.path.join(B, 'synofurb-mark-small.svg'))), strip(rd(os.path.join(B, 'synofurb-mark-small-dark.svg')))   # title bar is 20 px: the arrows-only small artwork
lg_l, lg_d = strip(rd(os.path.join(B, 'synofurb-logo.svg'))), strip(rd(os.path.join(B, 'synofurb-logo-dark.svg')))
def pair(l, d): return f'<span class="lg-l">{l}</span><span class="lg-d">{d}</span>'
logo_js = f"const LOGO={json.dumps(pair(mk_l, mk_d))};const LOGO_FULL={json.dumps(pair(lg_l, lg_d))};"
logo_css = '.lg-l,.lg-d{display:contents}.lg-d{display:none}:root[data-theme=dark] .lg-l{display:none}:root[data-theme=dark] .lg-d{display:contents}.lg-l svg,.lg-d svg{display:block;width:100%;height:100%}'
# ---- help topics from UX-SPEC.md (section 7, desktop topics)
spec = rd(os.path.join(ROOT, 'design', 'v2', 'UX-SPEC.md'))
sec = spec[spec.index('## 7. Help centre content'):spec.index('**Shortcuts tab (desktop)**')]
def inline(s):
    s = re.sub(r'&', '&amp;', s); s = re.sub(r'<', '&lt;', s)
    s = re.sub(r'`([^`]+)`', r'<code>\1</code>', s); s = re.sub(r'\*\*([^*]+)\*\*', r'<b>\1</b>', s); return s
def md(body):
    out, lines, i = [], body.strip().split('\n'), 0
    while i < len(lines):
        l = lines[i]
        if l.startswith('|'):
            rows = []
            while i < len(lines) and lines[i].startswith('|'): rows.append(lines[i]); i += 1
            rows = [r for r in rows if not re.match(r'^\|[-| ]+\|$', r)]
            cells = [[c.strip() for c in r.strip('|').split('|')] for r in rows]
            out.append('<table><thead><tr>' + ''.join(f'<th>{inline(c)}</th>' for c in cells[0]) + '</tr></thead><tbody>' + ''.join('<tr>' + ''.join(f'<td>{inline(c)}</td>' for c in r) + '</tr>' for r in cells[1:]) + '</tbody></table>'); continue
        if re.match(r'^\d+\. ', l):
            items = []
            while i < len(lines) and re.match(r'^\d+\. ', lines[i]): items.append(re.sub(r'^\d+\. ', '', lines[i])); i += 1
            out.append('<ol>' + ''.join(f'<li>{inline(x)}</li>' for x in items) + '</ol>'); continue
        if l.startswith('- '):
            items = []
            while i < len(lines) and lines[i].startswith('- '): items.append(lines[i][2:]); i += 1
            out.append('<ul>' + ''.join(f'<li>{inline(x)}</li>' for x in items) + '</ul>'); continue
        if l.strip():
            # review N4: no wall of text; at most two sentences per paragraph, the first one is the lead
            sents = re.split(r'(?<=[.!?])\s+(?=[A-Z0-9(])', l.strip()); step = 2
            for k in range(0, len(sents), step): out.append(f'<p>{inline(" ".join(sents[k:k + step]))}</p>')
        i += 1
    return ''.join(out)
help_ = []
for m in re.finditer(r'#### ([a-z-]+): ([^\n]+)\n(.*?)(?=\n#### |\Z)', sec, re.S):
    help_.append({'id': m.group(1), 'title': m.group(2).strip(), 'html': md(m.group(3)), 'text': re.sub(r'<[^>]+>', ' ', md(m.group(3)))})
assert len(help_) == 14, len(help_)
html = f'''<!doctype html><html lang="en" data-theme="light"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>synofurb desktop mockup v2 (not the real app)</title>
<style>{fonts_css}{tokens_css}{rd(os.path.join(SRC,'app.css'))}{logo_css}</style></head><body>{sprite}
<div id="stage"><div class="win" role="application" aria-label="synofurb mockup window"><div id="shell" style="display:contents"></div><div id="layer"></div><div class="toasts" aria-live="polite"></div>
<div class="mockmark" role="note">mockup – not the real app<button aria-label="Hide mockup mark" title="Hide"><svg class="ic s"><use href="#i-x"/></svg></button></div></div></div>
<script>const HELP={json.dumps(help_)};{logo_js}
{js}</script></body></html>'''
open(os.path.join(H, 'index.html'), 'w').write(html); print('index.html', round(len(html) / 1024), 'KB;', len(want), 'icons;', len(help_), 'help topics')
