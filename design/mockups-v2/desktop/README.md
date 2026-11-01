# synofurb desktop mockup v2 (clickable prototype)

Mockup, not the real app. Not approved by anyone. Open `index.html` in a browser (single file, no network).

## Use
- Click through: Find, Connect, Inspect, Goal, Plan, System, Confirm, Install, Done, Manage. Boot manager and Recovery are on Goal; Settings, Help (F1), About and the menu are in the title bar.
- Window is 1280x800 and reflows to 960x640. Light and dark follow the OS; Settings > General switches.
- Hidden dev menu (jump to any screen or state): Ctrl+Shift+D. The "mockup" mark in the title bar can be closed.
- URL parameters for fixed states (used by the screenshots): `?bare=1&theme=dark&unit=ds918|ds923|ds1621&gate=ready&step=12&fail=pre|post|postkeep&health=ok|rebuild|faildisk|svcdown|unreachable&mgmt=updating|failed|updated&dlg=<name>&sheet=help&topic=<id>&pop=<id>#<screen>` (full list: `screens.json`).

## Build and checks
```
python3 build.py             # src/*.js, src/app.css, tokens-v2.json, fonts, icons, UX-SPEC help topics -> index.html
node check_words.js          # visible words per screen state, budget 60 (--size=960x640 too)
node check_contrast.js       # computed text / border colours, light + dark, 4.5:1 text, 3:1 non-text
node check_flows.js          # real clicks and keys: main path, gate, tooltips, dialogs, sheet focus trap, update flow, no h-scroll
node shots.js [regex|all]    # screenshots -> design/screenshots/v2-mockup-desktop-<screen>[-960]-<light|dark>.png
```
Playwright at `/opt/node-tools/node_modules/playwright` and `/usr/bin/google-chrome` are assumed (override with `PW`, `CHROME`).
