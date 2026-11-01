# Model compatibility list

synofurb ships a list of Synology models with a status for each one, so the app can tell you how much trust the
hardware has earned before you start. The data is [`compat/models.json`](../compat/models.json); this page explains
what the statuses mean, how reports become entries, and how the app uses the list. The rules for contributors are in
[`compat/README.md`](../compat/README.md).

## Statuses

| Status | Shown as | Meaning |
|---|---|---|
| `verified` | VERIFIED | A maintainer, or two independent people, ran synofurb on this model, Debian installed and booted, and logs are on file for a current release. |
| `community` | COMMUNITY | At least one report exists. Not yet verified to the project's standard. |
| `untested` | UNTESTED | No report yet. The hardware class should work in theory (for example standard x86-64 PC hardware). Not a promise. |
| `known-issues` | ISSUES | Reports show something fails or only partly works. Read the notes. |
| `unsupported` | UNSUPPORTED | The hardware cannot run Debian 13 (PowerPC, 32-bit x86, ARMv5, SoCs without mainline Linux). synofurb refuses to install. |

The app always shows the status as a word, never only as a colour.

**Current state: no model is `verified`.** Exactly one model is `community`: the DS923+, from a third-party project
(`rylos/synodeb`) that booted Debian from the DOM using DOM boot entries. That report is labelled as third-party
everywhere and was not made with synofurb.

## What is in the list

Every model on Synology's own CPU page ([What kind of CPU does my Synology NAS have?][kb]) plus a few older ones from
the SynoCommunity wiki that the Synology page no longer lists (x09 and older, EDS14, NVR216, RS10815xs+). Per model:
series, year, platform codename (`apollolake`, `rtd1296`, ...), CPU, vendor, Debian architecture, boot method, stock
RAM, status, evidence and notes. Where each fact came from is recorded in
[`compat/source/synology-cpu-table.csv`](../compat/source/synology-cpu-table.csv) (fetch date and URL are in its header).

Things to know about the data:

* Synology's page lists models only up to the x10 series. Older models come from the SynoCommunity wiki, which calls
  itself "not complete"; for those rows the CPU is left empty rather than guessed.
* `boot` is generalised per platform (`dom` = Synology's internal flash module on x86 boards, `uboot` = Synology U-Boot on
  ARM boards, `unknown` where we have no basis). It has not been checked on every unit.
* `untested` x86-64 models are untested for real. "Expected to work in theory" is a statement about the CPU class only.
* The platform table in `crates/synofurb-core/src/platform.rs` treats `alpine4k` as arm64. Synology's table says the
  Annapurna AL-212 in the DS416/DS215+ is ARMv7, and the list follows Synology. A unit test names this disagreement.

[kb]: https://kb.synology.com/en-global/DSM/tutorial/What_kind_of_CPU_does_my_NAS_have

## How reports become entries

1. Someone runs synofurb on a unit and fills in the **Compatibility report** issue form
   (`.github/ISSUE_TEMPLATE/compat-report.yml`): model, DSM version, synofurb version, result, boot method, what worked,
   and the installer log.
2. A maintainer checks the log and adds an *evidence item* to the model in `models.json` (or the reporter opens a
   pull request that does so).
3. The model's status follows the evidence rules: one report makes it `community`; a maintainer run, or two independent
   successful reports with logs, make it `verified`; partial or failed reports make it `known-issues` (with the problem in
   `notes`).
4. CI-style checks are the unit tests in `compat.rs`: unique names, architecture consistent with the platform table,
   status and evidence consistent, nothing `verified` without evidence, and so on.

Status only changes through a reviewed pull request.

## How the app uses it

* `CompatDb::embedded()` is the list compiled into the binary, so lookups work offline from the first start.
* `CompatDb::refresh(cache_dir, url)` downloads the newest list from the repository (`main` branch) with `ETag` /
  `If-None-Match`, a 4 MiB size cap and full validation. Anything wrong (no network, HTTP error, corrupt JSON, newer schema
  than the app understands, a `community` entry without evidence) leaves the cache untouched. The request carries no
  information about your NAS, only the app version in the User-Agent.
* `CompatDb::load_best(cache_dir)` picks the cached list if it is valid and at least as new as the embedded one.
* `lookup(model_or_unique)` finds a model from DSM's `model` value or its `unique` value (`synology_apollolake_918+`),
  ignoring case, spaces and `plus`/`+` spelling (`ds918plus`). It returns nothing instead of guessing when a bare number
  could be a DS or an RS and the platform does not settle it.
* `status_for(&DeviceFacts)` combines the entry with live detection (`platform::classify`) into a verdict: status, entry,
  one-line headline, detail lines, and `blocking`. Live detection can only make the verdict stricter: hardware that cannot
  run Debian 13 is `unsupported` and blocking even if the list has never heard of the model. A model that is not in the
  list is `untested` with a note saying so. Conflicts between the list and the device (different platform, architecture or
  CPU vendor) are listed in the details.
* Only `unsupported` blocks. Every other status is information, not a gate.
