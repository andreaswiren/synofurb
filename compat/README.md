# Model compatibility list

`models.json` is the list synofurb looks up to tell you how well tested your NAS model is. It is embedded in the app
(`crates/synofurb-core/src/compat.rs`) and refreshed from this repository's `main` branch when you are online.
`schema.json` is the JSON Schema; the Rust test `compat::tests::embedded_db_obeys_schema_rules` enforces the same
rules (and a few more, such as unique model names).

Full explanation of how the app uses it: [`docs/COMPAT.md`](../docs/COMPAT.md).

## Files

| File | Purpose |
|---|---|
| `models.json` | The list. One object per model. Reviewed by hand in pull requests. |
| `schema.json` | JSON Schema for `models.json`. |
| `source/synology-cpu-table.csv` | Hardware facts copied from Synology's CPU page (source URL and fetch date are in the header). |
| `../scripts/gen-compat.py` | Rebuilds the hardware fields of `models.json` from the CSV; keeps `status` and `evidence`. |

Do not edit hardware fields (CPU, platform, RAM, ...) in `models.json` by hand. Fix the CSV and re-run
`python3 scripts/gen-compat.py` (`--check` verifies the committed JSON is in sync).

## Statuses

| Status | Meaning | Requirement |
|---|---|---|
| `verified` | The maintainers, or two independent people, ran synofurb on this model and Debian booted and worked. | Either one `kind: synofurb`, `result: success` item with `"maintainer": true`, or two such items from two different reporters. Every counted item needs a log link, and at least one must be on the current synofurb release. |
| `community` | At least one report exists. | At least 1 evidence item. A third-party report (another tool booted Debian on this model) is allowed but is labelled as such. |
| `untested` | No report yet. The hardware class should work in theory. | No evidence. |
| `known-issues` | Reports show something fails or only partly works. | At least 1 evidence item with `result: partial` or `fail`, and the problem in `notes`. |
| `unsupported` | The hardware cannot run Debian 13 (PowerPC, 32-bit x86, ARMv5, SoCs without mainline Linux). | The reason is in `notes`. |

`untested` is not a promise. It means nobody has told us either way. At the time of writing **no model is `verified`**.

## Evidence rules

An evidence item is one test run on one physical unit.

```json
{
  "date": "2026-11-01",
  "reporter": "github-handle",
  "kind": "synofurb",
  "synofurb_version": "0.1.0",
  "dsm_version": "7.2.2-72806",
  "result": "success",
  "notes": "UEFI-less boot from DOM, two disks, RAID1. Network up after install.",
  "log_url": "https://github.com/andreaswiren/synofurb/issues/123"
}
```

* `result`: `success` = Debian installed, booted from the intended disks and the NICs and disks worked after a reboot.
  `partial` = it installed or booted but something important did not work (state what). `fail` = it did not boot or the
  install aborted.
* `log_url` must be an `https://` link to the saved installer log (the log the app writes; attach it to the issue). A
  claim without a log counts for nothing.
* `kind: synofurb` means *this tool* did it. `kind: third-party` is for other tools (for example the DS923+ entry from
  the synodeb project); third-party items never count towards `verified`.
* Do not paste serial numbers, MAC addresses, IP addresses or passwords into notes or logs. Redact them.
* One person reporting twice does not make two independent reports.

## How to contribute a report

1. Run synofurb on your NAS (or try and fail; failures are valuable).
2. Open an issue with the **Compatibility report** form (`.github/ISSUE_TEMPLATE/compat-report.yml`) and attach the log.
3. A maintainer turns it into an evidence item (or you may open a pull request that edits `models.json` yourself:
   add the item to `evidence`, update `status` by the table above, and keep the JSON valid; run `cargo test -p synofurb-core compat`).

Status changes only ever happen in `models.json` through a reviewed pull request.
