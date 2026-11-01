# Management plane B: status-first top bar

> **Alternative, not revised after review.** The independent UX review covered only the recommended options (desktop Concept A, mgmt-a). This concept still shows the first-draft content (for example a Verified sample unit, a "disk-header backup", a boot flash listed as preserved). Do not treat its copy as a spec. Layout ideas only.

Open `index.html`; the top bar jumps between pages and states (including 390 px phone frames).

A dark top navigation plus an always-visible **status strip** (Healthy, "RAID 5 sync 38 % · 5 h 40 min left", host, uptime, temperature). The Dashboard leads with **Needs attention** (failing disk, pending updates) before any metric. Storage is one scrolling page (Volumes, Arrays, LVM, Disks) with a chip row as in-page navigation; edits (users) dock in a right-hand panel instead of a dialog; the rollback banner sits below the strip on every page.

Phones: the strip collapses to one line in the header, a bottom tab bar replaces the nav, and Needs attention is the first card.

Pros: the most important facts (is it healthy? is something running?) are never more than a glance away on any page, which is what a phone user needs at 2 a.m.; fewer clicks for storage work. Cons: less familiar than the sidebar, top nav of eight items overflows below about 900 px (needs a More menu), the persistent strip uses vertical space on small laptop screens.
