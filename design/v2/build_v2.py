#!/usr/bin/env python3
"""Builds design/v2/tokens-v2.json and design/v2/synofurb-theme.json (a gpui-component ThemeSet) from one palette definition.
Colours start from the already reviewed Direction-1 palette (design/mockups/ui/tokens.json) and add what the real gpui-component
widgets need (status foregrounds, danger fill re-derived for the dark theme, selection, scrollbar, switch).  Run: python3 design/v2/build_v2.py
Then: python3 design/v2/tools/contrast_v2.py   (must exit 0)."""
import json, math, os
HERE = os.path.dirname(os.path.abspath(__file__))
V1 = json.load(open(os.path.join(HERE, '..', 'mockups', 'ui', 'tokens.json')))

def _s2l(x): return x / 12.92 if x <= 0.04045 else ((x + 0.055) / 1.055) ** 2.4
def _l2s(x): x = max(0.0, min(1.0, x)); return 12.92 * x if x <= 0.0031308 else 1.055 * x ** (1 / 2.4) - 0.055
def lum(h): h = h.lstrip('#'); r, g, b = (int(h[i:i + 2], 16) / 255 for i in (0, 2, 4)); return 0.2126 * _s2l(r) + 0.7152 * _s2l(g) + 0.0722 * _s2l(b)
def cr(a, b): x, y = sorted((lum(a), lum(b)), reverse=True); return (x + .05) / (y + .05)
def rgb(h): h = h.lstrip('#'); return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))
def mix(a, b, t): ra, rb = rgb(a), rgb(b); return '#%02X%02X%02X' % tuple(round(x * (1 - t) + y * t) for x, y in zip(ra, rb))
def oklch(L, C, h):
    c = C
    while True:
        a, b = c * math.cos(math.radians(h)), c * math.sin(math.radians(h))
        l_, m_, s_ = L + .3963377774 * a + .2158037573 * b, L - .1055613458 * a - .0638541728 * b, L - .0894841775 * a - 1.2914855480 * b
        l, m, s = l_ ** 3, m_ ** 3, s_ ** 3
        v = (4.0767416621 * l - 3.3077115913 * m + .2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - .3413193965 * s, -.0041960863 * l - .7034186147 * m + 1.7076147010 * s)
        if all(-1e-4 <= x <= 1 + 1e-4 for x in v) or c <= 0: break
        c -= .002
    return '#%02X%02X%02X' % tuple(round(_l2s(x) * 255) for x in v)
def red_with_lum(target):
    best = None
    for i in range(300, 700):
        h = oklch(i / 1000, .2, 26); d = abs(lum(h) - target)
        if best is None or d < best[0]: best = (d, h)
    return best[1]

def palette(th):
    P = {k.split('.', 2)[2]: v for k, v in V1.items() if k.startswith(f'color.{th}.')}
    for k in ('danger_edge', 'scrim', 'log_wrn_bg', 'log_err_bg'): pass
    P.pop('danger_edge', None)
    dark = th == 'dark'
    if dark:  # round-1 review B9: hover/pressed must be visible. Dark danger is a LIGHT red with a near-black label (like the dark primary):
        # hover and pressed get lighter in clear steps (>= 1.18:1 each), label stays >= 4.5:1, fill stays >= 3:1 on every dark surface.
        P['danger'] = red_with_lum(0.220); P['danger_hover'] = mix(P['danger'], '#FFFFFF', .20); P['danger_pressed'] = mix(P['danger'], '#FFFFFF', .38)
    P['on_danger'] = oklch(.16, .02, 25) if dark else '#FFFFFF'
    for s, hue in (('ok', 150), ('warn', 75), ('err', 25), ('info', 255)):
        P[f'on_{s}'] = oklch(.16, .02, hue) if dark else '#FFFFFF'
        P[f'{s}_fg_hover'] = mix(P[f'{s}_fg'], P['fg'], .14); P[f'{s}_fg_active'] = mix(P[f'{s}_fg'], P['fg'], .26)
    P['selection'] = mix(P['accent_bg'], P['accent'], .16 if dark else .22)
    P['scrollbar_thumb'] = P['border_strong']; P['scrollbar_thumb_hover'] = mix(P['border_strong'], P['fg'], .30 if dark else .20)
    P['switch_off'] = P['border_strong']
    return P
L, D = palette('light'), palette('dark')

# ---- ThemeConfig colour keys (gpui-component 0.8.1 theme/schema.rs) -> token. "#hex" literals allowed.
MAP = {
 'background': 'canvas', 'foreground': 'fg', 'border': 'border', 'input.border': 'border_strong', 'ring': 'ring', 'caret': 'fg', 'selection.background': 'selection', 'overlay': '#000000/0.6',
 'muted.background': 'surface_2', 'muted.foreground': 'fg_2', 'accent.background': 'surface_2', 'accent.foreground': 'fg', 'popover.background': 'surface', 'popover.foreground': 'fg',
 'primary.background': 'accent', 'primary.foreground': 'on_accent', 'primary.hover.background': 'accent_hover', 'primary.active.background': 'accent_pressed',
 'secondary.background': 'surface_2', 'secondary.foreground': 'fg', 'secondary.hover.background': 'surface_3', 'secondary.active.background': 'surface_3',
 'button.background': 'surface', 'button.foreground': 'fg', 'button.hover.background': 'surface_2', 'button.active.background': 'surface_3',
 'button.primary.background': 'accent', 'button.primary.foreground': 'on_accent', 'button.primary.hover.background': 'accent_hover', 'button.primary.active.background': 'accent_pressed',
 'button.danger.background': 'danger', 'button.danger.foreground': 'on_danger', 'button.danger.hover.background': 'danger_hover', 'button.danger.active.background': 'danger_pressed',
 'danger.background': 'err_fg', 'danger.foreground': 'on_err', 'danger.hover.background': 'err_fg_hover', 'danger.active.background': 'err_fg_active',
 'success.background': 'ok_fg', 'success.foreground': 'on_ok', 'success.hover.background': 'ok_fg_hover', 'success.active.background': 'ok_fg_active',
 'warning.background': 'warn_fg', 'warning.foreground': 'on_warn', 'warning.hover.background': 'warn_fg_hover', 'warning.active.background': 'warn_fg_active',
 'info.background': 'info_fg', 'info.foreground': 'on_info', 'info.hover.background': 'info_fg_hover', 'info.active.background': 'info_fg_active',
 'link': 'accent_fg', 'link.hover': 'accent_hover', 'link.active': 'accent_pressed',
 'list.background': 'surface', 'list.hover.background': 'surface_2', 'list.active.background': 'accent_bg', 'list.active.border': 'accent', 'list.even.background': 'surface', 'list.head.background': 'surface_2',
 'table.background': 'surface', 'table.hover.background': 'surface_2', 'table.active.background': 'accent_bg', 'table.active.border': 'accent', 'table.even.background': 'surface', 'table.head.background': 'surface_2', 'table.head.foreground': 'fg_2', 'table.foot.background': 'surface_2', 'table.foot.foreground': 'fg_2', 'table.row.border': 'border',
 'tab.background': '#00000000', 'tab.foreground': 'fg_2', 'tab.active.background': 'surface', 'tab.active.foreground': 'fg', 'tab_bar.background': '#00000000', 'tab_bar.segmented.background': 'surface_2',
 'sidebar.background': 'sidebar', 'sidebar.foreground': 'fg', 'sidebar.border': 'border', 'sidebar.accent.background': 'accent_bg', 'sidebar.accent.foreground': 'accent_fg', 'sidebar.primary.background': 'accent', 'sidebar.primary.foreground': 'on_accent',
 'scrollbar.background': '#00000000', 'scrollbar.thumb.background': 'scrollbar_thumb', 'scrollbar.thumb.hover.background': 'scrollbar_thumb_hover',
 'progress.bar.background': 'accent', 'slider.background': 'accent', 'slider.thumb.background': 'accent', 'switch.background': 'switch_off', 'switch.thumb.background': 'surface', 'skeleton.background': 'surface_3',
 'accordion.background': 'surface', 'group_box.background': 'surface_2', 'group_box.foreground': 'fg', 'description_list.label.background': 'surface_2', 'description_list.label.foreground': 'fg_2', 'drag.border': 'accent', 'drop_target.background': 'accent_bg',
 'title_bar.background': 'surface_2', 'title_bar.border': 'border', 'status_bar.background': 'surface', 'status_bar.border': 'border', 'window.border': 'border_strong',
 'chart.1': 'accent', 'chart.2': 'info_solid', 'chart.3': 'ok_solid', 'chart.4': 'warn_solid', 'chart.5': 'err_solid', 'chart.bullish': 'ok_solid', 'chart.bearish': 'err_solid', 'chart.grid': 'border',
 'base.red': 'err_solid', 'base.red.light': 'err_bg', 'base.green': 'ok_solid', 'base.green.light': 'ok_bg', 'base.blue': 'info_solid', 'base.blue.light': 'info_bg', 'base.yellow': 'warn_solid', 'base.yellow.light': 'warn_bg', 'base.magenta': 'accent', 'base.magenta.light': 'accent_bg', 'base.cyan': 'info_solid', 'base.cyan.light': 'info_bg',
}
def resolve(P, v):
    if v.startswith('#'):
        if '/' in v: h, a = v.split('/'); return h + '%02X' % round(float(a) * 255)
        return v
    return P[v]
TYPE = [('title', 24, 32, 700, '-0.015em', 'text_2xl', 'Page title (at most 4 words)'), ('section', 16, 24, 600, '0', 'text_base + font_semibold', 'Card, dialog and drawer titles'),
        ('body', 14, 20, 400, '0', 'text_sm', 'Default UI text, buttons, tables, tooltips, alerts'), ('caption', 12, 16, 400, '0', 'text_xs', 'Tags, kbd, helper text, table captions (floor: nothing smaller)'),
        ('mono', 13, 20, 400, '0', 'mono_font.size', 'Hashes, IPs, serials, commands'), ('log', 12, 18, 400, '0', 'text_xs + font_family(mono)', 'Log lines')]
SIZES = {'control.xs': 20, 'control.sm': 24, 'control.md': 32, 'control.lg': 32, 'input.lg': 44, 'table.row.xs': 26, 'table.row.sm': 30, 'table.row.md': 32, 'table.row.lg': 40,
         'tab.underline.md': 36, 'tab.segmented.md': 32, 'tab.underline.lg': 44, 'progress.xs': 4, 'progress.sm': 6, 'progress.md': 8, 'progress.lg': 10, 'stepper.icon.md': 24, 'stepper.icon.lg': 32,
         'sidebar.width': 240, 'sidebar.collapsed': 48, 'dialog.width': 448, 'dialog.width.wide': 640, 'sheet.width': 350, 'sheet.width.wide': 480, 'tooltip.pad_x': 8, 'tooltip.pad_y': 2, 'popover.pad': 12, 'kbd.min_w': 20, 'kbd.pad_x': 4,
         'tag.pad_x': 10, 'tag.pad_y': 4, 'alert.pad_x': 16, 'alert.pad_y': 10, 'alert.gap': 12, 'safety_rail.width': 216, 'icon.sm': 14, 'icon.md': 16, 'icon.lg': 20, 'icon.xl': 24, 'icon.stroke': 2}
SCROLL = {'hit_area_px': 16, 'thumb_idle_px': 6, 'thumb_active_px': 8, 'inset_px': 4, 'min_thumb_px': 48, 'radius_px': 8, 'idle_fade_ms': 2000, 'mode.log_and_tables': 'Always', 'mode.pages_and_lists': 'Scrolling', 'track': 'transparent'}
RADIUS = {'sm': 4, 'md': 8, 'lg': 12, 'full': 999}
def tokens():
    t = {'$meta.name': 'synofurb tokens v2', '$meta.fonts': 'Inter (UI) and JetBrains Mono (code), bundled; Montserrat only if used for the wordmark (the logo outlines Inter SemiBold)', '$meta.icons': 'Lucide via gpui-kit-assets (see DESIGN-SYSTEM-V2.md section 5)',
         '$meta.theme': 'gpui-component 0.8.1 ThemeConfig: font.size 16 (rem), radius 8, radius.lg 12; keys under map.* name the colour token each ThemeConfig colour key takes'}
    for th, P in (('light', L), ('dark', D)):
        for k, v in P.items(): t[f'color.{th}.{k}'] = v
    t['scrim.alpha'] = 0.6
    t['font.ui'] = 'Inter'; t['font.mono'] = 'JetBrains Mono'; t['font.size_rem'] = 16; t['font.mono_size'] = 13
    for n, s, l, w, tr, g, u in TYPE: t[f'type.{n}.size_px'] = s; t[f'type.{n}.line_px'] = l; t[f'type.{n}.weight'] = w; t[f'type.{n}.gpui'] = g
    t['type.weights_bundled'] = '400, 600, 700'; t['type.features'] = 'tnum on all UI text'
    for k, v in RADIUS.items(): t[f'radius.{k}'] = v
    for k, v in SIZES.items(): t[f'size.{k}'] = v
    for k, v in SCROLL.items(): t[f'scrollbar.{k}'] = v
    t.update({'motion.fast_ms': 120, 'motion.normal_ms': 180, 'motion.slow_ms': 280, 'motion.dialog_ms': 250, 'motion.sheet_ms': 150, 'motion.tooltip_ms': 150, 'motion.popover_ms': 150, 'tooltip.delay_ms': 400})
    for k, v in V1.items():
        if k.startswith('accent.'): t[k] = v
    # one source of truth for the default accent: accent.purple.* is the palette above (round-2 review N9)
    for th, P in (('light', L), ('dark', D)):
        for k in ('accent', 'accent_hover', 'accent_pressed', 'on_accent', 'accent_bg', 'accent_fg', 'ring'): t[f'accent.purple.{th}.{k}'] = P[k]
    for k, v in MAP.items(): t[f'map.{k}'] = v
    return t
def theme_set():
    def cfg(name, mode, P, default):
        return {'is_default': default, 'name': name, 'mode': mode, 'font.size': 16, 'font.family': 'Inter', 'mono_font.family': 'JetBrains Mono', 'mono_font.size': 13, 'radius': RADIUS['md'], 'radius.lg': RADIUS['lg'], 'shadow': True,
                'colors': {k: resolve(P, v) for k, v in MAP.items()}}
    return {'name': 'synofurb', 'author': 'synofurb', 'themes': [cfg('synofurb Light', 'light', L, True), cfg('synofurb Dark', 'dark', D, False)]}
if __name__ == '__main__':
    json.dump(tokens(), open(os.path.join(HERE, 'tokens-v2.json'), 'w'), indent=1); open(os.path.join(HERE, 'tokens-v2.json'), 'a').write('\n')
    json.dump(theme_set(), open(os.path.join(HERE, 'synofurb-theme.json'), 'w'), indent=1); open(os.path.join(HERE, 'synofurb-theme.json'), 'a').write('\n')
    print('danger dark', D['danger'], round(cr('#FFFFFF', D['danger']), 2), 'on surface', round(cr(D['danger'], D['surface']), 2), 'canvas', round(cr(D['danger'], D['canvas']), 2))
