#!/bin/sh
# Render the demo screens of the desktop app headless (Xvfb + software Vulkan) and fail when content runs into the right-hand page
# margin or a horizontal scrollbar appears. This is the "no horizontal scroll or clipping" walk of the UI review (BRIEF-v2: no
# horizontal scrolling at any text size).
#
# What is walked, per run (one text size and one theme):
#   * every demo screen the binary lists (pages, dialogs, the Help sheet), first viewport;
#   * the same pages scrolled down by one and two viewports (mouse wheel through XTest);
#   * extra states that need an input: the log viewer open, the Copy command dialog, the Cancel install dialog and the
#     disabled-reason Popover.
#
#   crates/synofurb/assets/tools/gui-walk.sh [--text default|large|largest] [--theme light|dark] [--size 960x640]
#                                             [--keep DIR] [--no-scroll] [--no-extra] [screen...]
#
# Needs: Xvfb, ImageMagick (import, convert), python3 with Pillow, bc, a built binary at $SYNOFURB_BIN (default
# $CARGO_TARGET_DIR/debug/synofurb) and a Vulkan ICD (lavapipe: VK_ICD_FILENAMES). Never writes into the repository.
# The 2 px black strip at the left edge of every capture is the X root window (no window manager), not the app.
TEXT=default; THEME=light; SIZE=960x640; KEEP=""; SCROLL=1; EXTRA=1
while [ $# -gt 0 ]; do
  case "$1" in
    --text) TEXT=$2; shift 2;; --theme) THEME=$2; shift 2;; --size) SIZE=$2; shift 2;; --keep) KEEP=$2; shift 2;;
    --no-scroll) SCROLL=0; shift;; --no-extra) EXTRA=0; shift;;
    *) break;;
  esac
done
BIN=${SYNOFURB_BIN:-${CARGO_TARGET_DIR:-target}/debug/synofurb}
ICD=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.x86_64.json}
W=${SIZE%x*}; H=${SIZE#*x}
OUT=$(mktemp -d); trap 'rm -rf "$OUT"' EXIT
SCREENS=${*:-$("$BIN" gui --demo --screen list 2>&1 | sed -n 's/.*Choose one of: \(.*\) (the old.*/\1/p' | tr -d ',')}

# XTest helper: keys and the mouse wheel (args: wait:S move:X,Y click:X,Y scroll:N combo:Control_L+Shift_L+c)
cat > "$OUT/xk.py" <<'PY'
import ctypes, sys, time
x11 = ctypes.CDLL('libX11.so.6'); xt = ctypes.CDLL('libXtst.so.6')
x11.XOpenDisplay.restype = ctypes.c_void_p
d = ctypes.c_void_p(x11.XOpenDisplay(None))
def key(name, down):
    ks = x11.XStringToKeysym(name.encode()); kc = x11.XKeysymToKeycode(d, ks)
    xt.XTestFakeKeyEvent(d, kc, 1 if down else 0, 0); x11.XFlush(d)
def tap(name): key(name, True); time.sleep(0.05); key(name, False); time.sleep(0.25)
def move(x, y): xt.XTestFakeMotionEvent(d, -1, x, y, 0); x11.XFlush(d); time.sleep(0.2)
def button(b, down): xt.XTestFakeButtonEvent(d, b, 1 if down else 0, 0); x11.XFlush(d)
for a in sys.argv[1:]:
    if a.startswith('wait:'): time.sleep(float(a[5:]))
    elif a.startswith('move:'): x, y = a[5:].split(','); move(int(x), int(y))
    elif a.startswith('click:'): x, y = a[6:].split(','); move(int(x), int(y)); button(1, True); time.sleep(0.05); button(1, False); time.sleep(0.4)
    elif a.startswith('scroll:'):
        for _ in range(int(a[7:])): button(5, True); button(5, False); x11.XFlush(d); time.sleep(0.04)
        time.sleep(0.6)
    elif a.startswith('combo:'):
        ks = a[6:].split('+')
        for k in ks[:-1]: key(k, True)
        tap(ks[-1])
        for k in reversed(ks[:-1]): key(k, False)
    else: tap(a)
PY

# overflow verdicts for one capture: prints OVERFLOW lines (gutter / horizontal scrollbar), nothing when clean
analyse() { # $1 png  $2 label  $3 screen
  sd=$(convert "$1" -crop 7x$((H-330))+$((W-22))+210 +repage -colorspace Gray -format '%[fx:standard_deviation*255]' info: 2>/dev/null)
  bad=$(echo "${sd:-0} > 0.3" | bc -l 2>/dev/null || echo 0)
  case "$3" in help) bad=0;; esac   # the Help sheet is a panel over the right side: the gutter test does not apply
  hs=$(python3 -I - "$1" <<'PY'
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB"); w, h = im.size; px = im.load()
THUMB = {(128, 128, 128), (134, 134, 134)}
def runs(y):
    r, start = [], None
    for x in range(w):
        if px[x, y] in THUMB:
            start = x if start is None else start
        elif start is not None:
            if x - start >= 60: r.append((start, x))
            start = None
    if start is not None and w - start >= 60: r.append((start, w))
    return r
for y in range(100, h - 60):
    for a, b in runs(y):
        if all(any(a2 <= a + 3 and b2 >= b - 3 for a2, b2 in runs(y + d)) for d in (2, 3)):
            print("1"); sys.exit()
print("0")
PY
)
  [ "$hs" = "1" ] && bad=1
  if [ "$bad" = "1" ]; then
    echo "OVERFLOW $2 (gutter deviation $sd, horizontal scrollbar: $hs)"
    [ -n "$KEEP" ] && mkdir -p "$KEEP" && cp "$1" "$KEEP/walk-$2-$TEXT-$THEME.png"
    return 1
  fi
  return 0
}

n=0; fail=0
# run <label> <screen> <env> <xk args...>: one app run, one capture per entry of $SHOTS ("" = first viewport, then scroll steps)
run() {
  label=$1; screen=$2; envs=$3; shift 3
  D=$((300 + n % 90)); n=$((n + 1))
  Xvfb :$D -screen 0 ${W}x${H}x24 >/dev/null 2>&1 & XV=$!
  sleep 1
  env $envs DISPLAY=:$D HOME=$OUT VK_ICD_FILENAMES=$ICD WAYLAND_DISPLAY= SYNOFURB_WINDOW=$SIZE SYNOFURB_THEME=$THEME SYNOFURB_TEXT=$TEXT \
    timeout 90 "$BIN" gui --demo --screen "$screen" >/dev/null 2>&1 & APP=$!
  sleep 8
  [ $# -gt 0 ] && { DISPLAY=:$D python3 -I "$OUT/xk.py" "$@"; sleep 1; }
  v=0; res=0
  while :; do
    DISPLAY=:$D import -window root "$OUT/$label-$v.png" 2>/dev/null
    analyse "$OUT/$label-$v.png" "$label@$v" "$screen" || res=1
    v=$((v + 1))
    [ "$SCROLL" = 1 ] && [ $v -le 2 ] || break
    DISPLAY=:$D python3 -I "$OUT/xk.py" move:$((W / 2)),$((H / 2)) scroll:14
  done
  kill $APP $XV 2>/dev/null; wait $APP $XV 2>/dev/null
  [ $res = 0 ] && echo "ok       $label" || fail=1
}

for sc in $SCREENS; do run "$sc" "$sc" ""; done
if [ "$EXTRA" = 1 ] && [ -z "$*" ]; then
  run log-viewer install-after-ponr "SYNOFURB_DEMO_LOG=1"
  run command-dialog confirm-ready "" combo:Control_L+Shift_L+c wait:2
  run cancel-install install "" click:$((W - 55)),$((H - 63)) wait:2
  run reason-popover confirm "" click:$((W - 32)),$((H - 62)) wait:2
fi
exit $fail
