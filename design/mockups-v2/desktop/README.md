# synofurb desktop mockup v2 (clickable prototype, round 2)

Mockup, not the real app. Not approved by anyone. Open `index.html` in a browser (single file, no network).

## Use
- Click through the six wizard steps: Find, Connect (then Inspect), Goal, Setup (Storage and System tabs), Confirm, Install (then Done). Goal also has **Quick setup** (recommended defaults, then straight to Confirm). Boot manager and Recovery are on Goal; Manage opens after Finish or from "Running Debian" on Find; Settings, Help (F1, contextual), About and the menu are in the title bar.
- Window is 1280x800 and reflows to 960x640. All lengths are `rem`: **Settings > Text size** (Default 100 %, Large 115 %, Largest 130 %) scales the whole window. Light and dark follow the OS; Settings > General switches.
- Hidden dev menu (jump to any screen or state, including the Connect failures, the install interruptions, the clock outcomes and the Manage health states): Ctrl+Shift+D. The "mockup" mark in the title bar can be closed. A hidden **Components** page (dev menu) shows stepper, buttons in all states, tags, alerts, an open Select and the scrollbar.
- Demo behaviour: password `wrong` fails the sign-in; the install, the countdowns, the boot-agent install and the management update are simulated (`?fast=1` runs them 4x faster).
- URL parameters for fixed states (used by the screenshots): `?bare=1&theme=dark&size=largest&unit=ds918|ds923|ds1621|ds918b&twin=1&gate=ready|part&step=9&lost=1&fail=stopped|pre|post|postkeep&phase=waiting|back|notback|loader|lost&clock=2&run=1&conn=empty|login|off|unreach|hostchanged&boot=progress|result&facts=flash,disks,debian&health=ok|rebuild|faildisk|svcdown|unreachable&layout=hyb&mgmt=updating|failed|updated&dlg=<name>&sheet=help&topic=<id>&pop=<id>&select=<id>&reason=<css>#<screen>` (full list: `screens.json`).

## Build and checks (all must pass)
```
python3 build.py             # src/*.js, src/app.css, tokens-v2.json, fonts, icons, UX-SPEC help topics -> index.html
node check_words.js          # visible words per screen state, budget 60, prose inside "data" spans counted (--size=960x640 too)
node check_contrast.js       # computed text / border colours, light + dark, 4.5:1 text, 3:1 non-text
node check_scale.js          # type scale 12/14/16/24 (mono 12/13), weights 400/600/700, radii 4/8/12/pill
node check_flows.js          # real clicks and keys: main path, gate, Enter on Erase, twins, command == plan, clocks, recovery, boot manager, Manage, select, sheet focus trap, text size, one scroller, no h-scroll
node check_deadends.js       # clicks every enabled button of every screen: something visible must change
node shots.js [regex|all]    # screenshots (scrollbars shown) -> design/screenshots/v2-mockup-desktop-<screen>[-960|-960-130]-<light|dark>.png
```
Playwright at `/opt/node-tools/node_modules/playwright` and `/usr/bin/google-chrome` are assumed (override with `PW`, `CHROME`).
