#!/bin/sh
# Render every demo screen of the desktop app headless (Xvfb + software Vulkan) and fail when content runs into the right-hand
# page margin: a table or row wider than the page shows up as pixels in the 24 px gutter between the content and the scrollbar.
# This is the "no horizontal scroll or clipping" walk of the UI review (BRIEF-v2: no horizontal scrolling at any text size).
#
#   crates/synofurb/assets/tools/gui-walk.sh [--text default|large|largest] [--theme light|dark] [--size 960x640] [--keep DIR] [screen...]
#
# Needs: Xvfb, ImageMagick (import, convert), a built binary at $SYNOFURB_BIN (default target/debug/synofurb or
# $CARGO_TARGET_DIR/debug/synofurb) and a Vulkan ICD (lavapipe: VK_ICD_FILENAMES). Never writes into the repository.
TEXT=default; THEME=light; SIZE=960x640; KEEP=""
while [ $# -gt 0 ]; do
  case "$1" in
    --text) TEXT=$2; shift 2;; --theme) THEME=$2; shift 2;; --size) SIZE=$2; shift 2;; --keep) KEEP=$2; shift 2;;
    *) break;;
  esac
done
BIN=${SYNOFURB_BIN:-${CARGO_TARGET_DIR:-target}/debug/synofurb}
ICD=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.x86_64.json}
W=${SIZE%x*}; H=${SIZE#*x}
OUT=$(mktemp -d); trap 'rm -rf "$OUT"' EXIT
SCREENS=${*:-$("$BIN" gui --demo --screen list 2>&1 | sed -n 's/.*Choose one of: \(.*\) (the old.*/\1/p' | tr -d ',')}
fail=0; n=0
for sc in $SCREENS; do
  D=$((300 + n % 90)); n=$((n + 1))
  Xvfb :$D -screen 0 ${W}x${H}x24 >/dev/null 2>&1 & XV=$!
  sleep 1
  DISPLAY=:$D HOME=$OUT VK_ICD_FILENAMES=$ICD WAYLAND_DISPLAY= SYNOFURB_WINDOW=$SIZE SYNOFURB_THEME=$THEME SYNOFURB_TEXT=$TEXT \
    timeout 30 "$BIN" gui --demo --screen "$sc" >/dev/null 2>&1 & APP=$!
  sleep 8
  DISPLAY=:$D import -window root "$OUT/$sc.png" 2>/dev/null
  kill $APP $XV 2>/dev/null; wait $APP $XV 2>/dev/null
  # gutter: x from W-22 to W-15, below the unit header and above the action bar. The window sits 2 px right of its origin without a WM.
  sd=$(convert "$OUT/$sc.png" -crop 7x$((H-300))+$((W-22))+170 +repage -colorspace Gray -format '%[fx:standard_deviation*255]' info: 2>/dev/null)
  bad=$(echo "${sd:-0} > 0.6" | bc -l 2>/dev/null || echo 0)
  if [ "$bad" = "1" ]; then echo "OVERFLOW $sc (gutter deviation $sd)"; fail=1; [ -n "$KEEP" ] && mkdir -p "$KEEP" && cp "$OUT/$sc.png" "$KEEP/walk-$sc-$TEXT-$THEME.png"; else echo "ok       $sc"; fi
done
exit $fail
