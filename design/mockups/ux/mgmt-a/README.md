# Management plane A: sidebar app shell (revised after review)

Open `index.html`; the top bar jumps between pages and states, including 390 px phone frames. Sample data throughout.

ForgeVault's shell, shrunk: persistent left navigation, breadcrumb with `? Help`, version badge. Borrowed from mgmt-B and now built in: an always-visible **status strip** (worst health, running task, host, uptime), a **Needs attention** block at the top of the dashboard, and a **docked edit panel** (users, interface edit).

## What the review changed

* **Safe apply across a new origin**: before applying, the form shows the new URL, the certificate fingerprint (first 8 bytes + last 4, `Show full` for all 32), that the browser will warn again, that passkeys do not work on IP addresses, the 90 s limit and an SSH confirm. The banner repeats it with a timer; the login page at the new address shows the countdown; Apply is locked while pending; the rolled-back message never promises the old address.
* **Health is the worst component**: the dashboard says "Needs attention: 1 disk warning, 1 task running", not Healthy.
* **Boot page** is honest: power-cycling returns to the default; `kexec` leaves nothing of ours running; the DSM loader entry is shown and explained; making a non-Debian entry the default needs a typed phrase.
* **Login**: lockout remedy is wait or restart the service over SSH; says anyone on the LAN can trigger a pause; fingerprint check uses a long form.
* Create-volume is one click at the end; delete asks for a checkbox and a typed phrase; mobile network has DNS and IPv6.

Pros: familiar, easy to extend. Cons: eight nav items are heavy on a small box; the pending banner is tall on phones (consider collapsing after the first view).
