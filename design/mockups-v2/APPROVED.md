# Desktop mockup v2: owner approval

**Approved by the owner (Andreas Wiren) on 2026-11-01**, in the conversation: "Looks very good. Desktop mockup is approved."

This approves the **desktop app mockup** (round 6) as the design to implement in GPUI. It is *not* the release gate: the
UX and UI critics must still approve the **implemented** app (`docs/DESIGN-GATE.md`, `design/approvals/`), and that approval is
bound to a design hash that does not exist yet. Nothing in this file satisfies `scripts/release-gate.py`.

| Artefact | sha256 |
|---|---|
| design/mockups-v2/desktop/index.html | e52ffd670ffef063894a36ea969fae97ccbc500ff7e70155ce110f811c34abed |
| design/v2/UX-SPEC.md | d628ea7127e9aff697521aaa5f6c75024e19dd569757f4ef782e6cd6028a978f |
| design/v2/DESIGN-SYSTEM-V2.md | d11755a153e9925a141ff1880cef6d51718ca0d38b8f3c091251e9dfc109f926 |

Critic verdicts on this mockup (reviews in `design/reviews/v2/`): UI critic round 5 APPROVE, UX critic round 5 APPROVE,
UX expert round 4 no blocking findings. Round 6 (polish) was not re-reviewed by the critics; the owner approved it.

Decisions the owner accepted with the approval: six-step wizard with a merged Setup step, Quick setup on Goal, desktop Storage
read-only with an "Open web UI" hand-off, management-app update allowed with a warning while an array is degraded, twin-unit
phrase suffix, explicit Hybrid vs RAID 6 choice in Quick setup, the agent-did-not-answer states with manual recovery.

Known limits carried into implementation (mockup README, "Known limits to test in GPUI"): nothing was rendered by gpui-component;
24 px info and copy icon buttons; "Recommended" on Boot manager untested; Confirm needs a scroll at Largest text on 960x640;
Enter-on-Erase handling, Select placement, RadioGroup roving focus, Popover focus return and px widgets at Largest text are unverified.

## Changes after approval (owner sign-off pending)

**These changes were made by the implementation after the approval above. The owner has not signed off on them; this section records
them so the approved mockup is not mistaken for the product. Nothing here is an approval.** The file hashes above refer to the
approved versions, which are recoverable from git at commit `69eaff9` (where this file was created); `design/v2/DESIGN-SYSTEM-V2.md` section 13 has been extended since (DEV-UI-9 to DEV-UI-22) and `design/v2/UX-SPEC.md`
F3 was corrected.

| What changed | Why | Where |
|---|---|---|
| **No rescue panel on the Failed page** (DEV-UI-18). The approved mockup showed "Open rescue shell", an SSH command, a one-time password and rescue notes after the point of no return. The product has none: the installer has no rescue shell and no SSH server, so the page shows the log, Copy log, Export diagnostics, Recovery options and whether the unit reboots by itself (boot-flash units: 60 s before and 15 min after the point of no return; all others halt and wait for a power cycle). | The engine has no rescue shell (`docs/RECOVERY.md`); promising one would be false. | `crates/synofurb/src/ui/screens/failed.rs`, `facts.rs` (`fail_clock`), `copy.rs` (`DLG_NEXT_*`) |
| **Kernel default is Newest** (the mockup's Quick setup and System said Stable by default; "Stable" stays one click away). | Product requirement: kernel 7+ preferred; the engine, README and CLI default are Newest. | `app.rs`, `overlays.rs` (Quick setup), `cli_cmd.rs`, UX-SPEC F3 |
| Recovery actions run the engine (Boot DSM with an explicit reboot choice, Open agent, Restore flash with a unit-specific typed phrase, Remove entries); Restore uses only the unit's own saved image. | The mockup showed buttons; the engine now has the functions. | `screens/recover.rs`, `facts.rs`, `worker.rs` |
| Manage has its own sign-in and is bound to the unit on screen ("Sign in to read health"); Health, Update and Roll back never use the Connect form. | A release blocker found in review (wrong-unit actions). | `screens/manage.rs`, `mgmt.rs` |
| First-contact SSH host-key dialog shows the remaining answer time and says "Connection ended" when the attempt is over. | The engine's answer window is 180 s. | `overlays.rs`, `trust.rs` |
| Find columns share the table width instead of fixed widths; "Found by" ends in an ellipsis at large text. | No horizontal scrolling at Largest text. | `screens/inventory.rs` |
| The failure clocks are rules, not live counters: "DSM returns in 1 min", "DSM loader in 15 min" (boot-flash units only) and "DSM may return in 20 min or more" (DEV-UI-11). The mockup animated a countdown. | No timer-driven repaint, and the engine reports no seconds. | `facts::fail_clock`, `screens/failed.rs` |
| Management update and roll back run the engine against the unit on screen; the demo animation is only a demo (DEV-UI-13). Never run against a real unit. | The mockup only drew the buttons. | `screens/manage.rs`, `worker.rs` |
| No OS keychain: nothing offers to save a password (DEV-UI-14). | The core has no keychain API. | -- |
| At Largest text on 960 x 640 Confirm keeps two columns and scrolls; the hidden unit header and the reorder at 52 rem are not ported (DEV-UI-15). | Not implemented. | `screens/confirm.rs` |
| The install progress is a custom 12-segment strip with the "No return" flag over step 3 (DEV-UI-17). | No widget has segments or a flag. | `screens/install.rs` |
| The keyboard focus indicator is a solid 1 px accent line, not the 2 px ring with offset (DEV-UI-22). | The widget's ring rendered at 2.3:1 / 2.5:1. | `theme.rs` |


## Owner confirmation after implementation (2026-11-02)

After the implemented desktop app and the list of changes made since the mockup approval were summarised to the owner, the owner wrote:
"V2 is approved." This is recorded as the owner's approval of the v2 desktop design as implemented, including the changes listed in the
section above (no installer rescue panel, Newest kernel default, no keychain). If the owner meant only the mockup, say so and this
paragraph is corrected. It is not the release gate: `design/approvals/` still needs the critics' verdicts for a design hash, which
also needs Windows and macOS evidence.
