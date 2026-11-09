# Factory reset of a Synology unit

What to do when you cannot log in to DSM, do not know the unit's address, or want a unit emptied. Available without a unit:

```
synofurb-cli reset-help RS812+          # the procedures, the button location where it is recorded, the Download Center page
synofurb-cli reset-help DS918+ --json
synofurb-cli reset-help --list          # the models that have an entry
```

The code is `crates/synofurb-core/src/reset.rs`. **Nothing here was tried on a unit.** The desktop app does not show this yet (see "Not done").

## The three ways, least to most destructive

| | How | Use it when | Effect |
|---|---|---|---|
| **Mode 1** | Unit on. Paper clip in the RESET hole, hold about **4 seconds** until **one beep**, release at once. | You cannot log in or do not know the address. | Resets the IP address, the DNS server and the admin password to their defaults. Synology's guide for the button lists nothing else. |
| **Mode 2** | Unit on. Hold RESET about 4 seconds until a beep, release, press again **within 10 seconds** and hold about 4 seconds until **several beeps** (Synology's older text says three), release. | DSM does not start. | Reinstalls DSM and clears its system settings. The unit appears as "Not installed" in find.synology.com; install DSM with the `.pat` for the model. Needs a disk installed and a DSM that was set up before. |
| **System Reset** (DSM 7) | Control Panel, Update & Restore, System Reset, **Erase all data**. Needs a login. | The unit is to be emptied. | **Erases all data on the drives.** Not available in a High Availability cluster. |

Back up before any of them. Synology describes the two button resets as keeping the data on the volumes, but this project could not read that
article's text (see below): confirm it in the current article before you rely on it.

## Where the RESET button is

It is model specific. It is a pinhole or a button, **usually on the back**. The place is in the Hardware Installation Guide of the model, on its
Download Center page: `https://www.synology.com/en-global/support/download/<MODEL>` (for `DS918+`: `.../download/DS918+`).

* **RS812+: back panel.** Read in Synology's own quick installation guide (`Syno_QIG_RS8bay2_enu.pdf`, "Synology RackStation RS812+/RS812RP+ Quick Installation Guide", 23 pages), which lists
  "RESET Button, Back Panel: 1. To restore IP, DNS, passwords for the admin account to default value. 2. To reinstall the Synology RackStation."
  The RS812+ runs DSM 6.2.4 at most, so the DSM 6 procedure above is the one that applies. That guide does not give the press sequence; the sequence is from the article below.
* **Every other model: not recorded.** `reset-help` says so and gives the model's Download Center page instead of guessing.

## What is verified, and what is not

Checked on 2026-11-09:

* **The model list.** 78 models. Each was asked of Synology's Download Center (`support/download/<MODEL>`); a name Synology does not know is redirected to the generic page, and these did not
  redirect. Three names in the list we were given do not exist there and are **not** in the table: `DS122j`, `DS222+` and `DS2414+` (probably meant `DS2413+` or `DS2415+`, which exist and are not in our list; we did not add models nobody asked for).
  The years are the ones in the list we were given; `DS423` and `DS423+` appeared under both 2024 and 2023 and are 2023.
* **The button sequence** is Synology's wording as quoted by its product manuals and by the knowledge-base article
  ["How do I reset my Synology NAS? (For DSM 6.2.3 or earlier)"](https://kb.synology.com/en-global/knowledgebase/tutorials/493). The knowledge-base pages only render in a browser, so the article's own text could not be read here:
  the sequence was taken from search-result excerpts of Synology's pages (product manuals of the DS223j, DS124 and DS225+ and the article above) and agrees across them. **DSM 7** has its own
  [System Reset](https://kb.synology.com/en-global/DSM/help/DSM/AdminCenter/system_restoredefaults?version=7) page; the Erase all data path comes from the same excerpts.
* **Not verified:** the exact button location of every model except the RS812+; whether Mode 2 keeps the volumes on every model and DSM version; what newer DSM asks after a Mode 1 reset; the
  number of beeps (three in the older text); the 10-second window for each model. The RS812+ guide in particular says only what the button does.

## Not done

* **The desktop app** does not show this yet. A "Can't log in?" help on the Connect step and a link from Recover would be the places; it needs copy, a dialog, screenshots and the design gate.
* **Per-model button locations** for the other 77 models: each is in that model's Hardware Installation Guide PDF. Synology's download server has no listing, so the file names are not guessable
  (the RS812+ guide is `.../HIG/RackStation/12-year/RS812+/enu/Syno_QIG_RS8bay2_enu.pdf`); a person with each model's page open, or each PDF, can fill `button` in `reset.rs`.
* **Other models** (more RackStations, FlashStations, the J and SE series not in the list, older than 2014): add a line to `MODELS` after checking the Download Center page.
