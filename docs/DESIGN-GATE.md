# Design approval gate

**Rule:** no release build runs unless two independent, harsh reviewers - an expert **UX** critic and an expert **UI** critic -
have approved the exact design that is about to ship. The reviewers are Claude subagents defined in `.claude/agents/`; the
enforcement is `scripts/release-gate.py`, which is the first job (`design-gate`) of `.github/workflows/release.yml` and which
every build job depends on.

## The pieces

| Piece | File |
|---|---|
| UX critic (task success, destructive-flow safety, error recovery, IA, accessibility, copy) | `.claude/agents/ux-critic.md` |
| UI critic (consistency, WCAG contrast numbers, hierarchy, density, states, theming, Windows/macOS/Linux conformance, implementation fidelity) | `.claude/agents/ui-critic.md` |
| The gate (hashing, schema validation, override handling) | `scripts/release-gate.py`, tests in `tests/gate/` |
| Stored verdicts | `design/approvals/ux-critic-<hash>.json`, `design/approvals/ui-critic-<hash>.json` |
| Process summary next to the verdicts | `design/approvals/README.md` |

## The design hash

`design_hash` = SHA-256 over a deterministic listing `"<sha256 of file>  <path>\n"`, sorted by path, of exactly these files
(paths relative to the repository root, POSIX separators):

| What | Why it is part of the design |
|---|---|
| `design/mockups/**` (every file) | the withdrawn v1 mockups; still hashed so nothing changes silently |
| `design/mockups-v2/**`, `design/v2/**` | the owner-approved v2 desktop mockup, design system, tokens and UX spec |
| `design/DESIGN-SYSTEM*` (`DESIGN-SYSTEM.md` or a `DESIGN-SYSTEM*/` directory) | the rules the implementation is held to |
| `design/screenshots/**` (every file) | the evidence of what was actually built |
| `apps/mgmt-plane/**/*.html`, `*.htm`, `*.css`, `*.js`, `*.svg` (not under `tests/`) | stand-alone web UI files of the management plane |
| `apps/mgmt-plane/synofurb_mgmt.py#INDEX_HTML` | the management plane's UI is embedded in the Python file; the `INDEX_HTML` string constant is extracted with `ast`, so **backend-only edits do not invalidate approvals** but UI edits do |
| `crates/synofurb/src/ui/**` | the GPUI desktop UI sources |
| `crates/synofurb/src/gui.rs`, `crates/synofurb/src/gui/**` | the legacy/transition GUI, while it still exists |

Excluded everywhere: `__pycache__`, `*.pyc`, `.DS_Store`, `Thumbs.db`, and `design/approvals/**` itself. Text files (no NUL byte in
the first 8 KiB) are hashed with CRLF normalised to LF so a Windows checkout with `autocrlf` yields the same hash as Linux.
Symlinks are never followed (the link text is hashed). `python3 scripts/release-gate.py --list` prints every file with its hash and
the final `design_hash`; `--print-hash` prints just the hash.

**Why a hash?** An approval is a statement about a specific thing. If one pixel, one string or one flow changes after the review,
the statement is no longer true, and nothing - not a person remembering "it was approved last week" - may carry it over. Binding the
verdict to the content hash makes stale approvals unusable automatically, with no list of "files that matter" to keep in anyone's head.
Approvals also expire after **30 days** even if nothing changed, because reviews should reflect today's rules and tooling.

## What the gate checks

`python3 scripts/release-gate.py` exits 0 only if, for both `ux-critic` and `ui-critic`:

1. the file `design/approvals/<critic>-<design_hash>.json` exists for the **current** hash (approvals for other hashes are listed as stale and ignored);
2. it is valid JSON with `critic` equal to the critic, `verdict` = `approved`, `design_hash` equal to the current hash,
   `reviewed_at` an ISO-8601 timestamp **with a time zone**, not in the future (10 min skew) and not older than 30 days,
   `reviewer_model` a non-empty string;
3. `scores` contains every required key (UX: `task_success`, `destructive_safety`, `error_recovery`, `information_architecture`,
   `accessibility`, `copy`; UI: `consistency`, `contrast`, `hierarchy`, `density`, `states`, `theming`, `platform_conformance`,
   `implementation_fidelity`), each a number 0-10, and (for an approval) none below **6**;
4. `blocking_findings` is an **empty list**; every entry of `blocking_findings` / `non_blocking_findings` must be an object with a non-empty `title` and `evidence`;
5. there is something to approve: at least one file under `design/mockups/` or `design/mockups-v2/`.

Otherwise it exits 1 and prints a precise explanation, e.g. `missing design/approvals/ux-critic-<hash>.json` or
`approval is 41 days old (limit 30); run the critic again`. Options: `--print-hash`, `--list`, `--dry-run` (report only, exit 0),
`--max-age-days N`, `--root DIR`, `--approvals-dir DIR`, `--now ISO` (tests). In GitHub Actions it also writes `design_hash` and
`gate_status` to `$GITHUB_OUTPUT` and a summary to `$GITHUB_STEP_SUMMARY`.

**Today the gate fails, because no approvals exist. That is the intended state.**

## Running the critics

The critics need Claude Code with the Opus model available, started in the repository root. Run **each critic in its own invocation / session**
so that they cannot influence each other (they are also instructed never to read the other's verdict):

```
Use the ux-critic agent to review the current design and write its verdict file.
Use the ui-critic agent to review the current design and write its verdict file.
```

or mention them with `@ux-critic` / `@ui-critic`. Each critic: pins the hash, reads the mockups, design system, screenshots,
management-plane UI and GPUI sources, computes contrast ratios with Python instead of eyeballing, scores a fixed rubric, and writes
`design/approvals/<critic>-<hash>.json` (its only permitted write; it has Read, Glob, Grep and Bash only). It re-checks the hash at the
end and writes nothing if the design moved while it was reviewing.

Then check how the gate sees the files:

```sh
python3 scripts/release-gate.py            # exit 0 and "RELEASE GATE: PASSED" when both approve
```

and review `git status`: the only new files should be the two verdicts.

### Rules the critics follow (and you should enforce)

* **Zero blocking findings or no approval.** One blocking finding means `rejected`.
* **No self-approval.** A critic may not approve work that was revised after its rejection without a *fresh* full review; whoever fixes the
  findings must not be the critic, and the fix commit changes the hash anyway so the old verdict no longer matches.
* **No fabricated evidence.** Findings cite file + line/selector or screenshot name; unverifiable items go to `limitations` and are not counted as passes.
* **Missing evidence blocks.** A shipped screen (or a platform) without screenshots/implementation evidence is a blocking finding. In particular the
  UI critic requires screenshots of the *implemented* desktop app on Windows, macOS and Linux.
* A critic is an LLM: treat its verdict as a strong, structured review, not as proof. The gate proves that the review happened on this exact design, not that the review was right.
  A maintainer remains responsible for the release.

## Committing and storing verdicts

Verdicts are plain JSON in the repository, committed like code: `git add design/approvals/*.json`. History is the audit trail
(who ran it, which model, when). Old verdicts for superseded hashes can stay; the gate lists them as "stale (ignored)". Never edit a
verdict by hand: the gate cannot tell, but the git history and the maintainers can.

## Overriding the gate (maintainers only, loud, temporary)

For a genuine emergency (for example a security hotfix with no UI change while the critics are unavailable) a maintainer may release
**without** approvals by committing `design/approvals/OVERRIDE.md`. It is deliberately inconvenient:

```
# Design gate override
design_hash: <the exact current hash>
maintainer: Jane Maintainer <jane@example.org>
reason: <at least 30 characters: why, what changed, what makes this safe>
granted: 2026-11-01
expires: 2026-11-05
```

The gate accepts the file only if **all** hold: `design_hash` equals the current hash (so it covers exactly one design state and dies with
the next design edit); `maintainer` is `Name <email>`; `reason` has at least 30 characters; `expires` is today or later and at most **14 days**
after `granted`; and the **last commit that touched the file carries a verified signature**. Procedure:

1. `git config gpg.format ssh && git config user.signingkey ~/.ssh/id_ed25519.pub` (or use GPG), add the key to GitHub as a *signing key*.
2. Write `OVERRIDE.md` as above, `git add design/approvals/OVERRIDE.md`, `git commit -S -m "design gate override: <reason>"`, push.
3. The `design-gate` job asks the GitHub API whether that commit is `verified`; locally `git log --format=%G? -- design/approvals/OVERRIDE.md` must print `G`.
4. The gate then **passes with a loud banner** (`DESIGN GATE OVERRIDDEN BY A MAINTAINER`), the run's warnings and job summary repeat it, the release notes begin with a
   warning, and the `release` job re-checks the status. Review the override in the PR like any other change; remove the file after the release.

An unsigned, expired, mismatched or incomplete override is reported (`OVERRIDE.md is present but NOT valid`) and does not help.

## Tests

`python3 -m unittest discover -s tests/gate -v` covers hashing (determinism, CRLF, symlinks, exclusions, `INDEX_HTML` extraction), every rejection path
(missing, stale hash, rejected, blocking findings, expired, future, naive timestamp, bad scores, missing evidence), `--dry-run`, `--print-hash`, `--list`,
CI outputs, and the override rules (the signed-commit case needs git + ssh-keygen able to verify SSH signatures and is skipped otherwise).
