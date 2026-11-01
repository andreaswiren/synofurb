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
