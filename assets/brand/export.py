#!/usr/bin/env python3
"""Rasterises the brand SVGs with Chromium (Playwright) and assembles PNG sets, .ico, .icns, the macOS .iconset and packaging/icons.
Needs: python3 + Pillow, node + playwright (set PW=/path/to/playwright and CHROME=/path/to/chrome if not on the default path).
Run build.py first."""
import os, shutil, struct, subprocess, io
from PIL import Image
HERE = os.path.dirname(os.path.abspath(__file__)); ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
PNG = os.path.join(HERE, 'png'); ICONSET = os.path.join(HERE, 'synofurb.iconset'); PKG = os.path.join(ROOT, 'packaging', 'icons')
for d in (PNG, ICONSET, PKG): os.makedirs(d, exist_ok=True)
SIZES = [16, 24, 32, 48, 64, 128, 256, 512, 1024]
def render(jobs): subprocess.run(['node', os.path.join(HERE, 'render.js'), PNG] + jobs, check=True)
src = lambda n: os.path.join(HERE, n)
jobs = []
for s in SIZES:
    jobs.append(f'{src("app-icon-small.svg" if s <= 48 else "app-icon.svg")}:synofurb-icon-{s}.png:{s}')
for s in (16, 24, 32, 48, 64, 128, 256, 512, 1024):
    jobs.append(f'{src("app-icon-macos.svg")}:macos-{s}.png:{s}')
for s in (16, 24, 32, 256, 512):
    jobs.append(f'{src("synofurb-mark-mono-small.svg" if s <= 32 else "synofurb-mark-mono.svg")}:synofurb-mark-mono-{s}.png:{s}')
for s in (256, 512): jobs.append(f'{src("synofurb-mark.svg")}:synofurb-mark-{s}.png:{s}')
render(jobs)
def png(n): return os.path.join(PNG, n)
def ico(sizes, files, out):
    data = [open(f, 'rb').read() for f in files]; n = len(sizes)
    head = struct.pack('<HHH', 0, 1, n); off = 6 + 16 * n; ent = b''
    for s, d in zip(sizes, data):
        ent += struct.pack('<BBBBHHII', 0 if s >= 256 else s, 0 if s >= 256 else s, 0, 0, 1, 32, len(d), off); off += len(d)
    open(out, 'wb').write(head + ent + b''.join(data))
ISZ = [16, 24, 32, 48, 64, 128, 256]
ico(ISZ, [png(f'synofurb-icon-{s}.png') for s in ISZ], os.path.join(HERE, 'synofurb.ico'))
ico([16, 32, 48], [png(f'synofurb-icon-{s}.png') for s in (16, 32, 48)], os.path.join(HERE, 'favicon.ico'))
shutil.copy(src('app-icon-small.svg'), os.path.join(HERE, 'favicon.svg'))
# macOS iconset (iconutil -c icns synofurb.iconset) and a ready .icns
m = {'icon_16x16.png': 16, 'icon_16x16@2x.png': 32, 'icon_32x32.png': 32, 'icon_32x32@2x.png': 64, 'icon_128x128.png': 128, 'icon_128x128@2x.png': 256, 'icon_256x256.png': 256, 'icon_256x256@2x.png': 512, 'icon_512x512.png': 512, 'icon_512x512@2x.png': 1024}
for n, s in m.items(): shutil.copy(png(f'macos-{s}.png'), os.path.join(ICONSET, n))
def icns(out):
    chunks = b''
    for t, s in (('icp4', 16), ('icp5', 32), ('icp6', 64), ('ic07', 128), ('ic08', 256), ('ic09', 512), ('ic10', 1024), ('ic11', 32), ('ic12', 64), ('ic13', 256), ('ic14', 512)):
        d = open(png(f'macos-{s}.png'), 'rb').read(); chunks += t.encode() + struct.pack('>I', len(d) + 8) + d
    open(out, 'wb').write(b'icns' + struct.pack('>I', len(chunks) + 8) + chunks)
icns(os.path.join(HERE, 'synofurb.icns'))
# packaging/icons: keep the names referenced by packaging/Packager.toml
for n, s in (('32x32.png', 32), ('128x128.png', 128), ('128x128@2x.png', 256), ('512x512.png', 512), ('icon.png', 512)): shutil.copy(png(f'synofurb-icon-{s}.png'), os.path.join(PKG, n))
shutil.copy(os.path.join(HERE, 'synofurb.ico'), os.path.join(PKG, 'icon.ico')); shutil.copy(os.path.join(HERE, 'synofurb.icns'), os.path.join(PKG, 'icon.icns'))
for f in os.listdir(PNG):
    if f.startswith('macos-') and False: os.remove(png(f))
print('exported', len(os.listdir(PNG)), 'PNGs')
