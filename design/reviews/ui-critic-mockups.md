# UI critic review: synofurb visual mockups (design/mockups/ui)

Reviewer stance: harsh, evidence-driven, not a redesign. All screenshots were produced with Playwright/Chromium from the files on disk (`?screen=<id>&theme=<light|dark>`), at 1100x760, 390x800 and 1280x800. Screenshots were examined by eye; numbers below were recomputed by me with the WCAG 2.2 relative-luminance formula directly from the hex values in `tokens.json` (and for D2/D3 from the CSS custom properties), not copied from the documents.

Examined by eye: D1 Inventory, Goal, Storage plan, Confirm (light + dark, plus a forced "enabled destructive button" variant), Install (dark), Failure, W2 Dashboard (dark), W3 Storage (dark), W4 Network (light, 1280 and 390), W7 Mobile; D2 Confirm (light) and Install (dark); D3 Confirm (light + dark). **Not examined individually:** D1 Connect, Inspect, Done, Update dialog, Login, Users, Boot; D2/D3 other screens (they share structure with D1 and the same screen list).

---

## 1. Verdicts

| Direction | Verdict | One line |
|---|---|---|
| **Direction 1 (ForgeVault family), recommended** | **APPROVE WITH CONDITIONS** | The palette, structure and the destroyed/preserved gate are good enough to build on, but the mock never shows the enabled destructive button, was never rendered at its own 960x640 minimum (the Compatibility column is clipped there), the web plane does not reflow at 390 px (the safe-apply "Keep" button falls off screen), and the doc and the mock disagree on gradients, hatching, all-caps and sub-12 px type. Six blocking findings, below. |
| Direction 2 (Ops console) | **REJECT as the base** | Denser and fine in dark, but 9.5-10.5 px micro labels, 30 px controls, uppercase tracked mono and a cyan accent that is close to the info blue; it also breaks the ForgeVault-family constraint. Borrowable: status edges and the details switch, as RECOMMENDATION.md already says. |
| Direction 3 (Friendly appliance) | **REJECT as the base** | Best dark theme and best destructive-button contrast of the three (6.39:1 in dark vs D1's 4.63:1), but it leaves the ForgeVault family, 44 px controls and 16 px type do not fit a 960x640 window (Confirm is about 1100 px tall), and the warm cream canvas weakens amber-warning separation (1.05:1). |

I agree with the pick of D1 given the constraint "web UI must be in the ForgeVault family". I do **not** agree with D1's own scorecard (31/40, "Contrast and non-colour cues: 5", "Data-loss gate clarity: 5", "Cost in GPUI: 5"); see the score table.

---

## 2. Score table (Direction 1, 1 = poor, 5 = excellent; evidence in sections 4-6)

| Criterion | Score | Evidence in one line |
|---|:-:|---|
| Consistency (spacing / radius / type scale actually applied) | **3** | Radius (6/8/12) and control heights (28/36/42) are applied consistently. But the type scale is broken by 9.5, 10.5, 11, 11.5, 12.5, 15, 17 and 22 px one-offs; paddings 7/9/11/14/18 px are off the 2-4-6-8-12-16-20-24 scale; two reds (`danger` #C60014 and `err_solid` #E30117) coexist. |
| Hierarchy and scannability | **4** | Inventory and Install screens scan well: one h1, one primary, quiet neutrals. Confirm has five red elements competing (see B1). |
| Contrast (numbers) | **4** | Every declared pair passes; every real-world pair I added mostly passes. Exceptions: `fg_3` light 4.29:1 as placeholder (fails 4.5), dark danger label 4.63:1 (thin), light `ok/warn_solid` progress fills 2.79/2.62:1 on their track, dark `border_strong` on `accent_bg` 2.99:1. |
| State coverage | **2** | Only `.btn` has hover (4 selectors). Zero `:active` rules. Zero real `<input>`, zero `tabindex`: checkboxes, radio cards, inputs, rows, rail items, segmented buttons are `div`/`i`, so the one global `:focus-visible` rule never reaches them. No hover on rows/rail/radio cards, no button loading state, error state drawn only on the slider field. |
| Density and target sizes | **3** | Desktop targets all >= 24 px except the 20x20 checkbox glyph (label presumably hit area). Phone: hamburger is 36x36 and `btn` is 36 px, contradicting "44 px targets" (measured in W7). 12 px as the body floor for captions/badges plus 9.5-11 px in diagram/chips is small. |
| Colour independence of status | **4** | Strong: badge = icon + word; Untested is dashed (border `border_strong` 3.64:1). Weak spots: the other four badge borders are only 1.74-2.56:1 against the surface and all "solid", so the border does not discriminate them; warn/ok progress variants differ only by colour (plus the word beside them). |
| Dark theme quality | **4** | Not an inversion: accent, status text and tints are re-derived (e.g. `ok_fg` #6ED889 on #0F3118 8.04:1). Elevation by lighter surface works (canvas #0A0A0A / surface #171717 / surface_2 #202020). Negatives: dark danger button is a coral that visually weighs the same as the violet primary beside it (B1); `err_bg` #4F1A18 banner is heavy. |
| Typography | **3** | Mock uses a CSS system stack, not Inter/JetBrains Mono, so metrics shown are not what ships. Tabular numerals applied on only 2-3 selectors; the countdown title "rolling back in 0:47" and tile values are proportional and will jitter. Token font stack lacks macOS/Linux names and is meaningless to GPUI. |
| Destructive action treatment | **3** | Content is excellent (verb + object + count, trash icon, typed hostname, disabled until gate passes). Visual claim "red button is the loudest thing and only there" is untested and false as drawn (B1). |
| Iconography | **4** | lucide-style set, 2 px stroke, consistent; status glyphs distinct. Minor: chevron bullets on Goal cards read as "expandable", not as list bullets. |
| Platform conformance (Win/macOS/Linux) | **2** | Only a Windows-style title bar is drawn (and "v0.9.2 - Windows 11" is hard-coded in the rail). No macOS traffic lights/left controls, no Linux CSD, no scrollbars or overlay-scroll behaviour anywhere (`overflow:hidden`), no 125/150/200% scaling check. |
| Implementability in GPUI | **3** | Core is cheap (flat fills, 1 px borders, SVG). But the mock contains things the doc says are excluded (gradient hatch, SVG pattern, inset shadow, `color-mix`), and the two costly items (text input, accessibility tree) sit on the critical path of the gate. |
| Web-UI fit and ForgeVault resemblance | **3** | Same layout language (sidebar, breadcrumb, cards, 36 px controls) and the accent was derived correctly (I reproduced #7139CB from oklch(0.50 0.21 295)). But input borders are 3.6:1 solid vs ForgeVault's hairline `--input`, badges are 6 px rectangles vs ForgeVault's `rounded-full`, focus is a 2 px offset ring vs `ring-[3px] ring-ring/50`, and the 390 px reflow is not implemented. |

D2 / D3 (brief): D2 3 on touch/target size (30 px controls, min text 9.5 px), 4 on contrast (every pair passes; `fg_3` 4.75/5.10, dark danger 5.05). D3 4 on contrast (fg_3 4.63/5.15, dark danger 6.39), 5 on targets, 2 on fit with 960x640 and with ForgeVault.

---

## 3. Recomputed contrast table (Direction 1)

Method: WCAG 2.x relative luminance, ratio = (L1+0.05)/(L2+0.05), from the hex in `tokens.json`. Required: 4.5 text, 3.0 non-text. "Doc" = whether it matches DESIGN-SYSTEM.md section 3 to 2 decimals. Every ratio the document *declares* reproduces exactly (I recomputed all 66 per theme; no mismatches). The rows marked **not in doc** are pairs the design actually uses but `tools/contrast.py` never tests.

### 3.1 Text

| Pair (light / dark) | Light | Dark | Req. | Result | Doc |
|---|---:|---:|---:|---|---|
| body `fg` on `canvas` | 18.97 | 18.97 | 4.5 | pass / pass | match |
| body `fg` on `surface` | 19.80 | 17.18 | 4.5 | pass / pass | match |
| muted `fg_2` on `surface` | 7.81 | 7.81 | 4.5 | pass / pass | match |
| muted `fg_2` on `canvas` | 7.49 | 8.62 | 4.5 | pass / pass | match |
| muted `fg_2` on `surface_3` (hatch half, track) | 6.44 | 6.17 | 4.5 | pass / pass | not in doc |
| placeholder `fg_3` on `surface` | **4.29** | 4.54 | 4.5 (WCAG 1.4.3 has no placeholder exemption) | **FAIL light** / pass (0.04 margin) | numbers match; doc tests it at 3.0 and calls it "non-essential", which is not a WCAG exemption |
| `fg_3` on `surface_2` | **3.87** | **4.13** | 4.5 | **FAIL both** | not in doc |
| `fg_3` on `canvas` | **4.11** | 5.01 | 4.5 | **FAIL light** / pass | not in doc |
| `fg_3` on `accent_bg` | **3.74** | **3.54** | 4.5 | **FAIL both** | not in doc |
| text on accent: `on_accent` on `accent` | 6.67 | 6.30 | 4.5 | pass / pass | match |
| `on_accent` on `accent_hover` | 8.25 | 7.70 | 4.5 | pass / pass | not in doc |
| destructive label `on_danger` on `danger` | 6.16 | **4.63** | 4.5 | pass / pass (0.13 margin) | match |
| alt dark label: white on `danger` | - | 4.39 | 4.5 | fail (so the doc's near-black label is the only passing choice) | - |
| `on_danger` on `danger_hover` | 7.57 | 5.71 | 4.5 | pass / pass | not in doc |
| `accent_fg` on `surface` / `accent_bg` | 8.93 / 7.77 | 9.29 / 7.24 | 4.5 | pass | match |
| Disabled destructive label, actual mock: `err_fg` on hatch `surface_2`/`surface_3` | 7.10 / 6.49 | 8.35 / 7.26 | (exempt) | n/a, but see B1 | **mock differs from doc** (doc: `fg_2` label) |
| Disabled generic label `fg_2` on `surface_2` / `surface_3` | 7.04 / 6.44 | 7.10 / 6.17 | (exempt) | n/a | match (surface_3 not in doc) |

### 3.2 Compatibility badges (text on badge fill; icon uses the same `*_fg` colour in the CSS)

| Status | `*_fg` on `*_bg` light | dark | on `accent_bg` (badge inside selected row; text only) light / dark | Border at 50% of `*_solid` vs surface, light / dark (not required, shown because the doc leans on "solid border") | Doc |
|---|---:|---:|---|---|---|
| Verified (ok) | 7.80 | 8.04 | 7.65 / 7.89 | 1.83 / 2.71 | match |
| Community-reported (info) | 7.53 | 7.87 | 7.47 / 7.51 | 1.89 / 2.46 | match |
| Untested (`fg_2` on `surface_2`, dashed `border_strong`) | 7.04 | 7.10 | pass | border 3.64 / 3.84 (pass) | match |
| Known issues (warn) | 7.44 | 8.36 | 7.16 / 8.52 | 1.74 / 3.27 | match |
| Unsupported (err) | 7.06 | 7.21 | 6.86 / 7.16 | 2.56 / 2.18 | match |

All five badges pass 4.5:1 with large margin in both themes. The *icon* contrast with the solid colour would be lower (e.g. `warn_solid` on `warn_bg` 2.88:1 light, `ok_solid` on `ok_bg` 3.00), but the CSS draws badge icons in `*_fg`, so this is fine; do not "improve" fidelity by switching glyphs to `*_solid` in GPUI.

### 3.3 Log lines (log background is always dark)

| Pair | Light-theme panel | Dark-theme panel | Result | Doc |
|---|---:|---:|---|---|
| `log_fg` on `log_bg` | 13.99 | 15.05 | pass | match |
| debug / timestamp `log_dim` | 7.02 | 7.02 | pass | match |
| DBG label `log_dbg` | 7.57 | 7.58 | pass | match |
| INF `log_inf` | 10.13 | 10.90 | pass | match |
| WRN `log_wrn` | 11.43 | 12.30 | pass | match |
| ERR `log_err` | 8.25 | 8.88 | pass | match |
| `log_wrn` on its 12% wash | 9.08 | 10.17 | pass | not in doc |
| `log_err` on its wash (mock uses 16%; doc says 12%) | 6.35 | 7.14 | pass | not in doc; **mock/doc differ** |
| `log_dim` on warn / err wash | 5.58 / 5.41 | 5.81 / 5.64 | pass | not in doc |

Log colours are the best-engineered part of the palette. Note `log_dbg` (#9DA5B1) and `log_dim` (#9E9E9E) are nearly the same grey: the DBG label and the timestamp column are told apart by glyph and position only.

### 3.4 Non-text (3:1)

| Pair | Light | Dark | Result | Doc |
|---|---:|---:|---|---|
| input / outline-button / checkbox border `border_strong` on `surface` | 3.64 | 3.84 | pass | match |
| `border_strong` on `canvas` | 3.49 | 4.24 | pass | not in doc |
| `border_strong` on `surface_2` (table header, hover) | 3.28 | 3.49 | pass | not in doc |
| `border_strong` on `accent_bg` (unchecked box inside selected card) | 3.17 | **2.99** | pass / **fail by 0.01** | not in doc |
| focus ring `ring` on `surface` / `canvas` | 6.67 / 6.39 | 5.82 / 6.43 | pass | match |
| ring on `accent_bg`, `err_bg`, `warn_bg`, `surface_2`, `sidebar` | 5.81, 5.97, 6.04, 6.01, 6.22 | 4.54, 4.57, 4.46, 5.29, 6.18 | pass | not in doc |
| ring vs the adjacent fill of the primary / danger button | 1.00 / 1.08 | 1.00 / 1.43 | n/a: separated by the 2 px offset gap, which shows `surface` (6.67 / 5.82); acceptable only because of the offset | not discussed |
| ring if drawn *inside* the always-dark log panel | **2.82** | 6.58 | **fail in light theme** if the ring is ever drawn over `log_bg` | not in doc |
| `*_solid` icon/bar on `surface` (ok / warn / err / info) | 3.39 / 3.18 / 4.90 / 3.99 | 7.21 / 9.43 / 5.24 / 6.21 | pass | match |
| progress fill on `surface_3` track: ok / warn | **2.79 / 2.62** | 5.70 / 7.45 | **fail light** (mitigated by the % text) | not in doc |
| progress/slider fill `accent` on track; `err_solid` on track | 5.49; 4.04 | 4.60; 4.14 | pass | not in doc |
| track `surface_3` vs `surface` (empty bar visibility) | **1.21** | **1.27** | fail: an empty (0%) bar has no visible boundary | not in doc |
| selected row edge `accent` vs `accent_bg` | 5.81 | 4.54 | pass (the tint itself is 1.15 / 1.28 vs surface, so the 3 px edge carries selection) | partly |

### 3.5 Contrast-section verdict

* The document's claim "66 of 66 pairs pass; lowest text 4.63:1; lowest non-text 3.18:1" is **arithmetically correct and misleading**. It is true only for the pairs listed in `contrast.py`. The lowest *real* text ratio is `fg_3` on `surface_2`/`accent_bg` (3.54-3.87), and the lowest real non-text ratios are 2.62 (warn progress fill), 2.79 (ok progress fill), 2.82 (ring in log, light) and 2.99 (dark checkbox on tint).
* The "highest contrast of the three directions" claim holds for body text only. D1 is the **only** direction whose placeholder token fails 4.5:1 in light (D1 4.29, D2 4.75, D3 4.63) and has the **lowest** dark destructive-label ratio (D1 4.63, D2 5.05, D3 6.39).

---

## 4. Blocking findings

Severity: High = would cause a wrong build or a safety/usability failure; Medium = will cost rework or fail a stated requirement.

### B1 (High) The destructive button is never designed in its enabled state, and the "loudest, and only there" claim is false as drawn
* **Where:** `direction-1/index.html` screen `confirm`; also D2 and D3.
* **Evidence:** `grep 'btn dng'` finds exactly one occurrence per direction and it is `disabled aria-disabled="true"`. No screen, in any direction, shows the destructive button enabled, hovered, focused, pressed or loading. I forced the enabled state (`document.querySelector('.btn.dng').disabled=false`) to look at it; screenshots in my scratchpad `enabled-confirm-full-{light,dark}.png`. On that one screen red appears in: the banner (`err_bg` fill + `err_fg` text), the 4 px "Will be destroyed" card edge, four trash icons, the hostname chip, the "4 items" badge, the step-5 "point of no return" strip, and the disabled button's label (`.btn.dng[disabled]{color:var(--err-fg)}`, which is not in the spec; DESIGN-SYSTEM.md says a disabled label is `fg_2`). In dark the enabled button (#E23439 coral) and the primary "Back to plan" (#A37BFE) are both large saturated mid-luminance fills side by side; the red does not dominate. In light the violet primary is just as saturated as the red. DESIGN-SYSTEM.md principle 1 ("red fills ... only on the destructive button, the data-loss banner and errors") is also inconsistent with itself: the banner and the strip *are* red fills, so red is on the screen five times before the button is reachable.
* **Why it matters:** the whole product exists for this one control; the visual system's central promise is unverified.
* **Must change:** (1) add the enabled / hover / focus / pressed / loading states of the destructive button to the mock in both themes; (2) decide and state the hierarchy on the gate: which element is loudest (I recommend the *enabled* red button, with the banner and strip reduced to outline + tint and no extra red edge/icons, or else drop the claim); (3) remove `color:var(--err-fg)` from the disabled state or write it into the spec, and define a disabled state that is visibly disabled at 6-8:1 label contrast (today the only cue is hatch + dashed); (4) resolve dark-mode weight against the violet primary (RECOMMENDATION.md already proposes testing a darker red with white label; note white on #E23439 is 4.39:1, so it must also be darker than #E23439).

### B2 (High) The 960x640 minimum window was never rendered; the key column is clipped at 960 px
* **Where:** all ten desktop screens; reproducible on `inventory`.
* **Evidence:** every desktop mock is a fixed `.win{width:1180px}` and 750-1040 px tall (measured: Inventory 749, Confirm 976, Plan 953, Install 928 at 960 wide; ~1050 at 1180). Injecting `.win{width:960px}` on Inventory: the table is 881 px wide inside a ~690 px content column and the **Compatibility column, the point of the screen, is cut off** (screenshot `min960-inventory.png`; at 960 the Install log filter segmented control also extends past the content edge). At a 1100 px viewport the viewer clips the 1180 px window (stage `overflow-x:auto`), so a reviewer at a common laptop width sees a cropped Confirm button row. No screen shows a scroll state, scrollbar, or the pinned action bar over scrolling content, although DESIGN-SYSTEM.md section 5 promises exactly that; Plan and Confirm content is ~900-1000 px against a 640 px minimum height.
* **Must change:** produce the 10 desktop screens at 960x640 (and 1100x760 as default), including the collapsed 56 px rail promised in D1 README, a scrolled state with the pinned action bar, and the 130% text scale the doc requires "must reflow at 960 x 640". Cut the table to <= 5 columns or let it scroll with visible affordance.

### B3 (High) The web plane does not reflow to phone width; the safe-apply "Keep" button is off-screen
* **Where:** `direction-1/index.html` screens `network`, `dashboard`, `storage`, `users`, `boot` at 390 px.
* **Evidence:** screenshots `d1-390-light-network.png` and `d1-390-light-dashboard.png`. At 390 px the Network page's rollback banner puts "Keep these settings" (the lock-out safety control) outside the viewport; the "Edit eth0" card is cut to a sliver; Dashboard's second tile row stays 4 columns (CPU / Memory / CPU temp / Network are squeezed to ~60 px each with wrapped text) even though DESIGN-SYSTEM.md section 10 promises "2 columns below 900 px and single column below 480"; the Disks, RAID and volume tables are cut mid-column. The only phone layout is `W7 Mobile`, a separate hand-built `.phone` frame (390 px with a 10 px bezel, so 370 px content), which proves nothing about the same markup reflowing. In W7 the hamburger is 36x36 and buttons are 36 px high, contradicting the "44 px targets" annotation (measured).
* **Must change:** make the real web screens responsive in the mock (or state plainly that W2-W6 are desktop-only and design the phone variants of Network, Storage, Users, Boot), with the safe-apply banner stacking and both actions >= 44 px high, full width.

### B4 (Medium) Placeholder token fails WCAG 1.4.3 in light; the contrast checker's coverage hides it
* **Where:** `tokens.json` `color.light.fg_3` #7A7A7A (and `.inp .ph0`); `ui/tools/contrast.py` pair list.
* **Evidence:** 4.29:1 on `surface`, 4.11 on `canvas`, 3.87 on `surface_2`, 3.74 on `accent_bg` (section 3.1). The document tests it against 3.0 and justifies "non-essential hints"; WCAG 2.2 SC 1.4.3 has no such exemption for placeholder or hint text. Dark passes on `surface` by 0.04 only (4.54) and fails on `surface_2` (4.13) and `accent_bg` (3.54). Separately, `contrast.py` never tests: the disabled/hatch pairs, status text on `accent_bg`, `border_strong` on `accent_bg` (dark 2.99), progress fills on their track, ring vs `log_bg`, and log lines on their washes, so "66 of 66 pass" overstates assurance.
* **Must change:** darken light `fg_3` to >= 4.5:1 on `surface_2` (about #6B6B6B or darker) or forbid it for any text; restrict `fg_3` to non-text glyphs; extend `PAIRS` with every combination in section 3 and make the build fail on them. Because tokens will be code-generated into `theme.rs`, this must be fixed before codegen.

### B5 (Medium) The mock contradicts the design system on the things that decide GPUI cost and type scale
* **Where:** `direction-1/index.html` vs `DESIGN-SYSTEM.md` sections 1, 4, 5, 9.
* **Evidence (all reproducible with grep on the file):**
  * Principle 5 / section 9: "no gradients", "hatch is a web-only nicety", "no inset shadows". The mock: `.btn[disabled]{background:repeating-linear-gradient(135deg, ...)}` (visible on the Confirm and Install buttons, i.e. the **desktop** app), an SVG `<pattern>` hatch on the overflowing LV, `box-shadow:inset 3px 0 var(--accent)` for the selected-row edge, 15 `color-mix()` calls.
  * Section 4: "Never go below 12 px". The mock: `font-size:9.5px` (partition labels p1/p2 in the Plan diagram), `10.5px` (diagram mono, layer labels), `11px` (chip architecture tag, step timers, slider max label, phone tab labels, rail numerals), `11.5px` (fingerprint block); also 12.5, 15, 17 px one-offs.
  * Section 4: "sentence case everywhere; no all-caps in Direction 1". The mock has literal all-caps text: DISKS, PARTITIONS, LVM . FS, INSTALL / CONFIGURE / VERIFY phase captions, POINT OF NO RETURN, MANAGE (sidebar group), SMART OK, COMMANDS THAT WILL RUN (AS ROOT).
  * Section 4 table: tabular numerals for tiles and countdown. CSS applies `tabular-nums` to `.rail .n`, `.stp .si` and `.tbl td.n` only; tile `.v` values and the "rolling back in 0:47" heading are proportional and will shift every second.
  * Section 4 / tokens.json font stack (`Inter, ui-sans-serif, system-ui, "Segoe UI", Roboto`) differs from the mock stack (`ui-sans-serif, system-ui, -apple-system, "Segoe UI Variable", ...`), and the mock mono stack has no JetBrains Mono. The doc says Inter is bundled; every screenshot (including mine) is rendered in whatever the machine's `system-ui` is, so line lengths, the 960 px clipping and the 130% scale behaviour will be different (wider) with Inter.
* **Must change:** pick one source of truth. Either bring the mock into line with the doc (no gradient/pattern, sentence case, >= 12 px, child-div selected edge, tabular nums on every numeric style), or amend the doc and cost the extra GPUI work honestly. The 9.5-11 px diagram text needs a real decision (scale the diagram, or accept a 12 px floor and a bigger diagram).

### B6 (Medium) State coverage is not enough to be a visual spec
* **Where:** CSS and markup of D1 (`hover` count 4, `:active` 0, `[disabled]` 2, `focus-visible` 1, `<input>` 0, `tabindex` 0, `<button>` 92).
* **Evidence:** DESIGN-SYSTEM.md section 6 specifies hover, pressed, focus, disabled for buttons and a focus ring for inputs. The mock implements hover for four button classes only. Pressed is never styled. The text field, both checkboxes, radio cards, segmented control, table rows, rail items and sidebar nav are non-focusable `div`/`i` elements, so no focus ring is demonstrated on any gate control; the field's "focus" is a pre-baked `.inp.focus` class. There is no loading/in-progress state for a button (Apply, Scan again, Test connection), no hover for rows / rail / nav / radio cards, no "checkbox focus", no field "valid / complete" state on the typed-hostname field (it shows a focused field with a counter, but never the matched state that unlocks the button), no error variant of the hostname field (mismatch).
* **Must change:** a single states sheet (component x default / hover / pressed / focus-visible / disabled / error / loading, light and dark) for button (4 variants), input, checkbox, radio card, segmented control, row, rail item; plus the hostname field in empty / partial / match / mismatch states. Without it engineers will invent the states in GPUI.

---

## 5. Non-blocking findings

1. **Placeholder/hint hierarchy.** `.sub`, `.hint`, `.tbl .sm2` use `fg_2` (7+:1) at 12-13 px; with `fg_2` that high, secondary text barely recedes from primary (visible on Install step timers vs labels). Fine for accessibility, weak for hierarchy; consider a mid-grey around 5:1.
2. **Two reds.** `danger` #C60014 vs `err_solid` #E30117 (light), #E23439 vs #F4514F (dark): near-duplicate tokens with different roles. Visible side by side in the Confirm button row (button) vs the step-5 strip. Collapse or separate them clearly.
3. **Hostname field shows an error-style feel while the user is mid-typing.** "7 of 9 characters match" in neutral text is good; but the field is drawn with a double ring (outline + 1 px offset), which reads as error/focus ambiguity. Specify one focus ring, offset 2 px as in the spec (the `.inp.focus` uses offset 1 px, so it already deviates from `focus.ring_offset: 2`).
4. **Paste policy conflict.** DESIGN-SYSTEM.md and the mock say "Paste is disabled on purpose"; RECOMMENDATION.md accessibility risk 2 says allow paste. Pick one; the visible helper text in the mock currently teaches the blocked-paste behaviour.
5. **Log filter counts.** Install shows "Debug 212 / Info 148 / Warn 2 / Error 0" summing to the 362 lines total, but the doc defines the segments as cumulative minimum levels (Info should then be 150). Failure screen: "Warn 2 / Error 2" while the visible excerpt shows two WRN and one ERR. Make counts and the rule agree.
6. **Level glyphs.** `· DBG`, `● INF`, `▲ WRN`, `✖ ERR` are Unicode characters, not icons; their metrics differ per font and per OS (the `✖` and `▲` come from fallback fonts on Windows and macOS). Use drawn SVG glyphs, as the doc does for every other icon.
7. **Stepper status for failure.** The rail's failed step uses `err_bg` row + `!` circle: good. But it also drops the "current" accent treatment, so the user loses "where am I in the sequence" on the failure screen; add the step name in the title.
8. **Goal cards.** Chevron icons as bullet markers imply expandable rows; use dots or check/minus glyphs. The preselected card is the destructive "Install Debian 13" (violet radio card + red "Erases all disks" badge): visual preselection of the erasing option deserves a deliberate decision (UX owner).
9. **Banner family vs ForgeVault.** The accent banner (`acc`) and selected-row tint share `accent_bg`; on the Network page the warn banner, "unsaved changes" badge and "1" nav badge all use amber, three amber objects in one viewport. Fine, but watch the "calm by default" principle.
10. **Phone frame shows an iOS-style 5G/battery status bar and notch bezel**: marketing furniture, no design value; use a neutral frame so the 390 px layout can be checked at true width.
11. **Window title bar is always Windows-style** (controls right, 46 px wide cells), "Windows 11" hard-coded in the rail footer. Need macOS (left traffic lights, 28-38 px title bar, system font SF), and Linux (GNOME/KDE client-side decorations vary, Wayland vs X11) variants; the doc admits this is "Medium" cost but does not draw it.
12. **No scrollbar design.** All containers are `overflow:hidden`. Windows 11 uses thin overlay scrollbars that expand; macOS hides them by default; GPUI scrolling is custom. Log, table, Storage plan and Confirm all scroll at 640 px height. Specify scrollbar thickness, thumb colour (`border_strong`? 3:1) and keyboard scroll.
13. **Slider.** 20 px thumb is fine, but the track is 6 px with no label per step; "dashed marker for maximum" is a 2 px dashed `fg_2` line at 4.3-4.5:1 on its backdrop; ok. The invalid state recolours the whole filled track red (`err_solid` on track 4.04:1), good; add a visible text "Too large" next to it (the mock has it, in the card header; keep it adjacent to the slider).
14. **Dark `border_strong` on tinted selected controls** 2.99 (checkbox/radio unchecked on `accent_bg`); bump dark `border_strong` by one step or use `fg_2` for that one case.
15. **Progress track visibility.** `surface_3` on `surface` is 1.21:1 (light) / 1.27:1 (dark): an empty bar is invisible. Add a 1 px `border_strong` or darker track. Light ok/warn fills are below 3:1 on the track (2.79 / 2.62): add a darker `*_solid` for bars.
16. **ForgeVault accent switcher claim ("all accents may stay on the web").** Using ForgeVault's own oklch values converted to sRGB (I verified the converter by reproducing #7139CB from oklch(0.50 0.21 295)): purple .55 vs white 5.39, blue 4.90, rose 4.81, green **3.99 (fails 4.5 for a white button label)**, amber **2.77 (fails 3:1 for ring and fill vs white)**. The doc admits only violet was verified; add these to the checker, or cap the web accent choices to those that pass. (Pre-existing ForgeVault issue, but synofurb inherits it.)
17. **Typography on OS fonts.** `tokens.json` `font.ui` has no `-apple-system`/"SF Pro", no "Helvetica Neue"/Arial, and `system-ui` is a CSS keyword that GPUI will not resolve. Since the plan is to bundle Inter + JetBrains Mono, fine, but state the weights shipped: the type scale uses weights 620 and 650 (variable-font values); a static Inter bundle would round these to 600/700 and shift widths.
18. **ForgeVault resemblance details.** ForgeVault (`components/ui/button.tsx`, `badge.tsx`, `globals.css`): h-9 default, `rounded-md` (8 px), badges `rounded-full`, focus `ring-[3px] ring-ring/50`, inputs `border-input` (#E5E5E5 hairline, 1.26:1), dark borders are white at 10%/15% alpha, Inter with `font-feature-settings: "cv11","ss01","cv02"`, accent-derived hover is `bg-primary/90`. D1 matches radius and heights, diverges on badge shape (6 px rectangles), focus ring style, input border weight (3.6:1 solid), and opaque dark borders. The family resemblance is real at screenshot scale and weaker component by component; fine if intended, but then document it as "synofurb web is a *derivative* of ForgeVault's tokens", not "the ForgeVault look". The WCAG-driven heavier input border is a defensible improvement; propose it upstream.
19. **Dark-mode shadows.** `e1`-`e3` are black shadows; the doc says "nearly invisible, which is fine". True, but then dialog separation in dark depends on the 1 px `border_strong` outline and the scrim; verify the Update dialog (not examined) at 55% scrim over `#171717` surface.
20. **Mobile bottom bar.** Tab items measure 51 px high (pass) but 11 px labels (below 12 px floor, B5).

---

## 6. What is genuinely good (brief, evidenced)

* **Destroyed vs preserved layout** (`confirm`): two parallel cards with trash/shield icons, concrete details ("sda, sdb, sdc, sdd", "5.3 TiB used . 7 shared folders"), and a "3 px edge + icon + word" status language. Scannable in dark and light, and colour-independent by construction (icon + word + edge).
* **Colour tokens are real design work**, not a mechanical invert: separate `*_fg` (text), `*_solid` (glyph/bar) and `*_bg` (tint) per status; every badge pair is 7.0-8.4:1 in both themes; log colours 7.0-15.1:1 including on the warning/error washes (5.4-12.5:1).
* **Compatibility badges** (`inventory`): five distinct icons, words, and a dashed outline only for Untested, with a legend on the same screen. By icon, word and border style they should remain distinguishable without colour (reasoned from the markup; I did not run a greyscale or colour-blind simulation).
* **Input and checkbox borders were raised to 3.6:1/3.8:1** (`border_strong`), beyond what ForgeVault itself does.
* **Opaque-hex-token discipline** with one-to-one mapping into a generated `Palette` struct is the right call for GPUI, and the derivation from ForgeVault oklch is correct (I reproduced #7139CB to the digit).
* **Install screen**: point-of-no-return dashed rule, phase captions, a cancel button that is disabled *with the reason in the action bar* ("Cancel is unavailable after the point of no return ...").
* **Failure screen**: states plainly the NAS is not bootable, ranks recovery as radio cards, labels the data-destroying option ("Erases Debian") and keeps it unselected.

---

## 7. GPUI implementability risks

I could not verify GPUI behaviour against a pinned revision (no sandbox access to the source). Treat the following as risks to be confirmed with a one-day spike, ordered by impact.

1. **Accessibility tree (critical).** The brief says GPUI Kit claims AccessKit support, unverified; DESIGN-SYSTEM.md section 9 says there is none. These two statements disagree, so nobody knows the baseline. Everything in the mock that says `role=progressbar`, `aria-live`, `aria-pressed`, `role=alert` (43 `role=`, 63 `aria-label` in D1) is web-only until proven otherwise. Do the spike first: a focusable button, a text field, a checkbox and a live region, driven by NVDA, Narrator and VoiceOver.
2. **Text input (critical path of the gate).** No ready-made input is assumed; the typed hostname, SSH user/password, port and IP fields all need caret, selection, clipboard, IME, undo, a password variant and (decide!) paste. The design shows a field with a "counter" and a match state; budget it as the largest widget.
3. **Text selection and copy.** Fingerprints, device paths, IPs, MACs, commands and log lines are mono data users must copy. The mock has Copy buttons only on the log. If GPUI text is not selectable, every fingerprint and path needs a copy affordance, and the design has none for the 11.5 px fingerprint block, the "Including the config backup saved to ~/synofurb/..." path, or the generated commands block.
4. **Things the mock uses that the doc excludes** (B5): `repeating-linear-gradient` button hatch, SVG `<pattern>` hatch, inset box-shadow for the selected row, `color-mix()` alpha tints. GPUI can draw gradients and shadows in recent revisions, and dashed borders exist, but combining dashed border + radius + hatch fill + clipping on a 36 px control is exactly the kind of thing that renders differently per backend (Metal / DirectX / Vulkan). Replace by: dashed border + opaque `surface_2`, no hatch.
5. **Storage diagram connectors.** Orthogonal connector lines between measured boxes (drawn in the mock as one SVG) need a custom `canvas()` layer with bounds measurement, reflow on resize and at 130% text scale; 9.5-10.5 px text in it breaks the 12 px floor. Medium to costly, as the doc says.
6. **Sparklines and rings.** `canvas()` + path is fine; the 60 px countdown ring is web-only (acceptable).
7. **Fonts.** Bundled Inter + mono: add the weight caveat (item 17 above), `font_features` for tabular numerals must be exposed by the pinned GPUI (unverified), and line height 1.3/1.5 are fractional in px (e.g. 21 px at 14 px is exact, h1 29/24 is not). Inter is wider than Segoe UI Variable/SF; the 960 px clipping in B2 gets worse.
8. **Scrolling and pinned bars.** One scroll container per screen with a sticky action bar is cheap; but kinetic scroll feel, overlay scrollbars and keyboard scroll differ across OSes and are a custom job (see non-blocking 12).
9. **Window chrome.** Custom titlebar with per-OS controls: Windows snap layouts (hover on maximise), macOS traffic lights and full-screen, Linux CSD/shadows/resize handles on Wayland. The mock draws only the Windows look.
10. **High-contrast / forced colours.** Custom drawing ignores Windows Contrast Themes and macOS Increase Contrast; RECOMMENDATION.md lists it; no palette is drawn for it.
11. **Focus order and focus ring primitives.** `.focus_visible()` exists in GPUI (to be verified), but the focus ring of 2 px with 2 px offset needs an outline outside the element box (not in layout flow) and clipping by parent `overflow:hidden` containers (the cards use `overflow:hidden` in the mock): rings on edge controls will be cut. Reserve ring space in parents.

---

## 8. What I could not verify

* Rendering in GPUI (no source or build available to me); the "Simple / Medium / Costly" table in DESIGN-SYSTEM.md section 9 is unchecked by me.
* Whether GPUI Kit's AccessKit support exists, works on all three OSes, or is usable for focus/live regions.
* Real fonts: all screenshots use the sandbox's `system-ui`, not Segoe UI Variable, SF Pro or the intended bundled Inter and JetBrains Mono, so line breaks and exact heights are only indicative.
* Windows/macOS/Linux window chrome, scaling at 125/150/200%, HiDPI text rendering, scrollbars.
* Safari/Firefox rendering of the web plane (the mock uses `color-mix()`); real screen readers; colour-blindness simulations (I reasoned from icons, words and borders; did not run a simulator).
* The screens listed as not examined in the preamble (Connect, Inspect, Done, Update dialog, Login, Users, Boot), and D2/D3 beyond Confirm and one Install, though D2/D3 contrast was recomputed from their CSS variables (all declared pairs pass; D2 min text pair `fg_3` 4.75 / 5.10, D3 4.63 / 5.15).
* Touch behaviour: target sizes were measured from layout boxes only.
* Any user testing; none of the scores are user-validated.

---

## 9. Minimum conditions to turn APPROVE WITH CONDITIONS into APPROVE

1. B1 closed: the enabled / hover / focus / pressed destructive button exists in both themes and the red hierarchy of the gate is decided and measured.
2. B2 closed: 960x640 renders of all desktop screens with scroll and collapsed rail; Inventory's Compatibility column visible.
3. B3 closed: the web plane reflows at 390 px, with the safe-apply Keep/Roll back controls reachable at >= 44 px.
4. B4 closed: `fg_3` >= 4.5:1 on every surface that carries it (or banned for text); checker extended to all real pairs.
5. B5 closed: one source of truth for gradients/hatch, type floor, all-caps, tabular figures and fonts.
6. B6 closed: a states sheet.
7. A GPUI spike result on accessibility and the text input exists before screens are built.
