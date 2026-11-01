# design/approvals

This directory holds the **design approvals** that unlock a release build. The rule of the product: before ANY release build,
two independent, harsh reviewers - the **ux-critic** and the **ui-critic** (Claude subagents defined in
`.claude/agents/`) - must have approved the *exact* design that is about to ship.

```
design/approvals/
  README.md                        this file
  ux-critic-<design_hash>.json     verdict of the UX critic for one exact design state
  ui-critic-<design_hash>.json     verdict of the UI critic for the same design state
  OVERRIDE.md                      (normally absent) a maintainer's signed, time-limited override - see docs/DESIGN-GATE.md
```

`<design_hash>` is the SHA-256 that `python3 scripts/release-gate.py --print-hash` prints. It covers `design/mockups/**`,
`design/DESIGN-SYSTEM*`, `design/screenshots/**`, the management plane's web UI, and the GPUI sources under
`crates/synofurb/src/ui/` (exact list: `python3 scripts/release-gate.py --list`). **Change any of those files and the hash
changes, so every existing approval stops applying** - there is nothing to "refresh" by hand; the critics must look again.

## Process

1. Finish the design work and commit it (screenshots of the implemented app included).
2. `python3 scripts/release-gate.py --print-hash` - note the hash.
3. In Claude Code, from the repository root, run the two critics as **separate** invocations (they must not see each other's output):
   * `Use the ux-critic agent to review the current design and write its verdict file.`
   * `Use the ui-critic agent to review the current design and write its verdict file.`
4. If either rejects: fix every blocking finding (a different person/agent than the critic), commit, and go back to step 2. The old
   verdict files are now stale (different hash); delete them or leave them as history.
5. When both files exist and the gate prints `PASSED`, commit them: `git add design/approvals && git commit -m "design: approvals for <hash12>"`.
6. Tag the release (`docs/RELEASING.md`). The release workflow re-computes the hash and re-checks the files; any drift fails the run.

## What the gate accepts

Both files must: be named with the current hash; contain `"critic"` matching their name, `"verdict": "approved"`,
`"design_hash"` equal to the current hash, `"reviewed_at"` (ISO-8601 with time zone) within the last **30 days** and not in the
future, a `"reviewer_model"`, all required `"scores"` (UX: task_success, destructive_safety, error_recovery,
information_architecture, accessibility, copy; UI: consistency, contrast, hierarchy, density, states, theming,
platform_conformance, implementation_fidelity - each 0-10, none below 6), and an **empty** `"blocking_findings"` list. Every
finding (blocking or not) needs a `title` and `evidence`. Full schema: `docs/DESIGN-GATE.md`.

There are no approvals in this directory until the critics have actually run. **The gate failing on a fresh checkout is the
intended state.**
