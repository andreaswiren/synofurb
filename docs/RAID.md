# RAID, SHR and array management

Status: **specification** shared by the installer planner (`synofurb-core`), the CLI/GUI and the NAS web UI
(`apps/mgmt-plane`). Implementation notes and verification results are added to the bottom as each part lands.

## Synology Hybrid RAID (SHR) is mdadm + LVM

SHR is not a separate RAID level. DSM builds it from ordinary Linux md arrays and LVM:

1. Each disk gets a small system partition (RAID1 `md0`) and a swap partition (`md1`), then one or more **data partitions**.
2. The disks' data space is cut into **slices** at the sizes of the different disks. For disks of 4, 4 and 8 TB there are
   two slices: 4 TB on all three disks, and a second 4 TB slice that exists on the 8 TB disk only.
3. Every slice that exists on several disks becomes one md array (`md2`, `md3`, ...): RAID1 for two members, RAID5 for three or
   more (SHR-1); RAID6 for four or more (SHR-2).
4. All those arrays are joined as physical volumes of one LVM volume group (`vg1000`), and the volume is one logical volume
   (`lv`) on top, formatted btrfs or ext4.

So the part of DSM that matters here is plain `mdadm` and `lvm2`. A Debian system can **read, assemble and mount** an existing SHR
volume (`mdadm --assemble --scan`, `vgchange -ay`), and `synofurb` can **build the same layout** for new volumes.

Honest limits:

* A layout built by `synofurb` is *SHR-style* (same technique, Debian-managed). It is not guaranteed to be adopted by DSM
  (DSM keeps its own metadata and volume registry), and DSM-created arrays use features (btrfs with Synology patches, `syno`
  superblock flags) that Debian does not fully understand. Imported DSM volumes are therefore read-only by default.
* Nothing in this repository has run on a real Synology.

## `synofurb` data layouts

| Choice | Meaning | Min disks | Fault tolerance |
|---|---|---|---|
| Auto | pick from disk count (existing behaviour) | 1 | depends |
| None / 0 / 1 / 5 / 6 / 10 | one md array over equal-size partitions (smallest disk size is used on every member) | 1 / 2 / 2 / 3 / 4 / 4 | as usual |
| **SHR-1 ("Hybrid, 1 disk")** | slices per above, RAID1/RAID5 per slice | 2 | any 1 disk |
| **SHR-2 ("Hybrid, 2 disks")** | slices per above, RAID6 per slice, RAID1 (n-way) for slices with fewer than 4 members | 4 | any 2 disks (for slices with 4+ members; see warnings) |

### Slice algorithm (pure function, unit-tested)

Input: data-partition size of every data disk. Round sizes down to whole GiB; tiers closer than 1 GiB are merged into the lower one.

1. Sort the distinct sizes ascending: `t1 < t2 < ... < tk`. Slice `j` has length `t_j - t_{j-1}` (with `t0 = 0`) and members = the disks whose size is `>= t_j`.
2. Slice with 1 member: unusable (reported as "unprotected capacity left unused", never silently used).
3. SHR-1: 2 members -> RAID1; >= 3 members -> RAID5. SHR-2: >= 4 -> RAID6; 2 or 3 -> RAID1 (3-way mirror when 3) and a warning
   that this slice tolerates fewer failures than requested.
4. Usable bytes per slice: RAID1 = length, RAID5 = (m-1) x length, RAID6 = (m-2) x length.
5. Partitions are created per disk consecutively; slice `j` on a disk is that disk's `j`-th data partition. Arrays are named `data1`, `data2`, ...
   in slice order, with md metadata 1.2. All arrays are PVs of `vg_data`; the data LV is linear over them.

Worked example (must be a test): 4 TB, 4 TB, 8 TB, SHR-1 -> slice 1: 4 TB on 3 disks, RAID5, 8 TB usable; slice 2: 4 TB on 1 disk, unusable. Total 8 TB (what DSM reports).
Example 2: 2, 4, 4, 8 TB SHR-1 -> slices 2 TB x4 (RAID5, 6 TB), 2 TB x3 (RAID5, 4 TB), 4 TB x1 (unused) -> 10 TB.
Example 3: 1, 2, 2, 3 TB SHR-1 -> 1 TB x4 (3), 1 TB x3 (2), 1 TB x1 (unused) = 5 TB.

## Array management in the NAS web UI and CLI

The web UI gets a **RAID** page (see `design/v2/UX-SPEC.md`) and the same operations are plain API routes, so a shell user can use them too.
Every destructive or long-running operation uses the existing pattern: server-side plan -> typed confirmation -> ordered steps -> audit.

| Operation | mdadm / LVM underneath | Notes |
|---|---|---|
| List arrays | `mdadm --detail`, `/proc/mdstat`, `/sys/block/mdX/md/*` | level, state, size, members, spares, bitmap, sync/reshape progress and speed, mismatch count, last check |
| Create array | `mdadm --create` | any level above, spares optional, SHR wizard builds slices + arrays + `vg_data` |
| Add spare | `mdadm --manage --add` | spare on a healthy array |
| Replace a failed member | `--fail` / `--remove` / `--add` | rebuild progress visible |
| Replace a healthy member | `--add` spare then `--replace X --with Y` | keeps redundancy during the swap |
| Grow capacity | `--grow --raid-devices=N` (RAID5/6), `--grow --size=max` (after replacing with larger disks), then `pvresize`, `lvextend`, `xfs_growfs` | reshape needs a backup file only for some levels |
| Change level | `--grow --level=...` | RAID1->RAID5 (>=3 members after add), RAID5->RAID6 (+1 disk), RAID0->... listed only where mdadm supports it |
| Scrub | write `check` or `repair` to `sync_action`; stop with `idle` | schedule monthly via systemd timer (Debian's `mdcheck` units) |
| Stop / assemble | `mdadm --stop`, `--assemble --scan` | refuse while mounted or in use by LVM |
| Tune | `speed_limit_min/max` | per-array `stripe_cache_size`, write-mostly, bitmap on/off |
| SHR add disk / replace disk | per slice: partition the new disk like the others, then add to each slice's array or create new slices | planned as a list of steps the user reviews |
| Mail on degrade | `mdadm --monitor` via `mdmonitor.service` | status shown on the dashboard |

Foreign arrays (created by DSM or another system) are shown and can be assembled and scrubbed. Create/grow/destroy on them require an explicit
"I understand this is not mine" confirmation.

Safety rules for every operation: never touch a device that is mounted, holds `/`, `/boot` or swap, or belongs to a different array;
refuse when the array is already resyncing/reshaping (except monitoring and `idle`); show the exact commands before running; re-derive the plan
on the server at execution time; sizes and names pass the existing validators.
