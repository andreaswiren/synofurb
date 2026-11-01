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
approved versions; `design/v2/DESIGN-SYSTEM-V2.md` section 13 has been extended since (DEV-UI-9 to DEV-UI-20) and `design/v2/UX-SPEC.md`
F3 was corrected.

| What changed | Why | Where |
|---|---|---|
| **No rescue panel on the Failed page** (DEV-UI-18). The approved mockup showed "Open rescue shell", an SSH command, a one-time password and rescue notes after the point of no return. The product has none: the installer has no rescue shell and no SSH server, so the page shows the log, Copy log, Export diagnostics, Recovery options and whether the unit reboots by itself (boot-flash units: 60 s before and 15 min after the point of no return; all others halt and wait for a power cycle). | The engine has no rescue shell (`docs/RECOVERY.md`); promising one would be false. | `crates/synofurb/src/ui/screens/failed.rs`, `facts.rs` (`fail_clock`), `copy.rs` (`DLG_NEXT_*`) |
| **Kernel default is Newest** (the mockup's Quick setup and System said Stable by default; "Stable" stays one click away). | Product requirement: kernel 7+ preferred; the engine, README and CLI default are Newest. | `app.rs`, `overlays.rs` (Quick setup), `cli_cmd.rs`, UX-SPEC F3 |
| Recovery actions run the engine (Boot DSM with an explicit reboot choice, Open agent, Restore flash with a unit-specific typed phrase, Remove entries); Restore uses only the unit's own saved image. | The mockup showed buttons; the engine now has the functions. | `screens/recover.rs`, `facts.rs`, `worker.rs` |
| Manage has its own sign-in and is bound to the unit on screen ("Sign in to read health"); Health, Update and Roll back never use the Connect form. | A release blocker found in review (wrong-unit actions). | `screens/manage.rs`, `mgmt.rs` |
| First-contact SSH host-key dialog shows the remaining answer time and says "Connection ended" when the attempt is over. | The engine's answer window is 180 s. | `overlays.rs`, `trust.rs` |
| Find columns share the table width instead of fixed widths; "Found by" ends in an ellipsis at large text. | No horizontal scrolling at Largest text. | `screens/inventory.rs` |
