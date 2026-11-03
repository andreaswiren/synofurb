# Certificates and the HAProxy front (Stage 2 design)

Status: **implemented** (Stage 2 of `docs/ADDONS.md`, management plane `0.6.x`, release `0.2.4`), **never run against a real Let's Encrypt, DNS zone, lego, HAProxy or systemd unit**: see "Implementation" at the end for what exists, what was tested and what is unverified. The text above that section is the design it was built from (two details changed while building it and are corrected in place: the store paths and the TLS terminator).

## Goal

One HAProxy fronts the management plane and every add-on web UI (Portainer, Frigate, Plex, ...). The management plane is the single place that
issues, stores, renews and publishes **several certificates** (one per hostname or per name group). Let's Encrypt, three challenge types, automatic renewal.

## The challenge types

| Type | Needs | How it is answered |
|---|---|---|
| **DNS-01** via `challenges.addr.tools` | one CNAME per name, a secret | see below; works behind NAT, allows wildcards; no inbound port |
| **HTTP-01** | the name resolves to this box, port 80 reachable from the Internet | HAProxy forwards `/.well-known/acme-challenge/` to a tiny local responder in the management plane (webroot style) |
| **TLS-ALPN-01** | the name resolves to this box, port 443 reachable | HAProxy sends connections with ALPN `acme-tls/1` to a local responder that serves the validation certificate |

**Tool choice (owner decision 2026-11-03: "use lego for everything").** One ACME client, `lego` (Debian trixie package `lego` 4.9.1-2 for amd64, arm64 and armhf; verified on packages.debian.org, installed by
the management plane with apt when the user enables certificates), handles all three challenges: DNS-01 (through its `exec` provider, see below), HTTP-01 (its built-in HTTP provider on a local port that HAProxy forwards
`/.well-known/acme-challenge/` to) and TLS-ALPN-01 (its built-in TLS provider on a local port that HAProxy sends `acme-tls/1` ALPN connections to). certbot is not used (it cannot do TLS-ALPN-01).
The management plane stays standard-library Python and drives `lego` and `haproxy` only through `subprocess` with fixed argv (no shell). Facts to re-verify on a unit with `lego --help` (4.9.1): flag names for the
HTTP/TLS provider ports, the DNS resolver and propagation options, `renew --days`, and the CNAME behaviour below.

**Does lego support challenges.addr.tools?** Not as a named provider (checked against lego's documented provider mechanism 2026-11-03). Two generic providers exist: `httpreq` (it POSTs JSON `{fqdn, value}`: does not match addr.tools'
`secret` + `txt` form fields) and `exec` (it runs a program: `present <FQDN> <value>` and `cleanup <FQDN> <value>`, default mode; exit code != 0 aborts; env `EXEC_PATH`, `EXEC_PROPAGATION_TIMEOUT` (default 60 s),
`EXEC_POLLING_INTERVAL` (3 s), `EXEC_SEQUENCE_INTERVAL` (60 s)). So the management plane ships a tiny adapter (`/usr/local/sbin/synofurb-acme-addr-tools`, generated, 0700, root) used as `EXEC_PATH`: on `present` it POSTs
`secret` and `txt=<value>` to `https://challenges.addr.tools` and succeeds on HTTP 201 `OK`; `cleanup` does nothing (records are temporary). The secret is passed to the adapter through a root-only file, never argv or the environment of
other processes. lego's propagation check queries the TXT record after following the `_acme-challenge` CNAME (lego follows CNAME delegation by default; re-verify the exact option name on the unit) into the addr.tools zone.

## DNS-01 through challenges.addr.tools (from https://challenges.addr.tools, read 2026-11-03)

* One-time DNS setup per name: `_acme-challenge.<name>  CNAME  <sha224>.challenges.addr.tools.` where `<sha224>` is the lowercase 56-hex SHA-224 of the **secret**
  (`echo -n "secret" | shasum -a 224 | cut -c 1-56`). A wildcard and its base name share one `_acme-challenge` record.
* Publishing a challenge: `POST https://challenges.addr.tools` with form fields `secret=<secret>` and `txt=<acme validation value>`; the service adds a TXT record (temporarily) at
  `<sha224>.challenges.addr.tools`; success is HTTP 201 with body `OK`. A request with only `secret` returns the subdomain name and changes nothing (use it to show the user the exact CNAME target).
* The secret is the only credential: the hash is public in DNS, so the secret must be strong (generate 32+ random characters by default); URL-encode it; never log it; store it 0600; show it once.
* The service documents no lifetime, rate limits or error bodies: the client must wait/poll for the TXT to be visible (`dig`/resolver check against the authoritative path) before telling the ACME server, retry with backoff, and treat anything other than 201 as a failure with the response text shown.
* lego command (what the app runs): `lego --server <LE url> --email <e> --accept-tos --path /etc/synofurb/lego --dns exec --domains <name> [--domains '*.name'] run|renew` with `EXEC_PATH=/usr/local/sbin/synofurb-acme-addr-tools`; one lego account per Let's Encrypt environment (production, staging).

The settings page asks for the secret (or generates one), shows the derived CNAME target per name with a Copy button and a "Check DNS" button that resolves `_acme-challenge.<name>` and reports whether it points at the right target.

## Certificate store and HAProxy

* Store: `/etc/synofurb/certs/<id>/{fullchain.pem,privkey.pem,meta.json}` (0600/0700, root); HAProxy gets combined PEMs in `/etc/haproxy/certs/` (one file per certificate; HAProxy serves the right one by SNI through a generated `crt-list`).
* Several certificates: a certificate has a primary name, optional SANs (and a wildcard), a challenge type, the add-ons/hostnames that use it, status (valid until, last renewal, last error). The UI lists them like the add-ons page: cards, status Tags, actions Renew now / Change / Remove.
* HAProxy config is generated by the management plane (a TCP frontend on 443 that splits off ALPN `acme-tls/1` and passes everything else to a loopback TLS terminator; the terminator picks the certificate by SNI and the backend by Host: the management plane and each add-on's `web` backend on localhost; frontend 80 for HTTP-01 and a redirect to https), validated with `haproxy -c -f` before every reload and rolled back on failure (the same safe-apply pattern as forgevault's `lib/system/haproxy.ts`; check `/home/user/forgevault` read-only for reference, never modify it).
* Renewal: a systemd timer (twice a day, random delay) renews certificates within 30 days of expiry, reloads HAProxy only when a certificate changed, records the result, and the dashboard shows expiring/failed certificates. Staging server option for testing (Let's Encrypt staging) and rate-limit awareness (5 duplicate certificates per week).
* Add-on hostnames: each add-on's `web.hostname_hint` becomes a hostname the user maps to a certificate; the add-on keeps listening on localhost/its port and HAProxy terminates TLS (Frigate's own 8971 https can be re-encrypted or bypassed: per-add-on `backend_scheme`).

## Out of scope for the first cut

DNS providers with their own APIs (Cloudflare, ...), ACME accounts for other CAs, mTLS, OCSP stapling controls.

## Implementation (what exists, 2026-11-03)

One new module, `apps/mgmt-plane/synofurb_edge.py` (standard library only, no import from `synofurb_mgmt`; part of the update bundle, the desktop app's embedded copy and the installer payload), plus one delimited glue block in
`synofurb_mgmt.py` (section 17c), the two web pages (script, copy and help live in the module and are injected into the single-page UI) and the demo/audit tooling.
`apps/mgmt-plane/tools/make-update-bundle.py`, `scripts/package-mgmt.sh`, `crates/synofurb-core/src/{manage,payload}.rs`, `payload/chroot-setup.sh` list the file; a test (`tests/test_edge.py`, `BundleContractTests`) checks every place.

### Files and permissions

| Path | What | Mode |
|---|---|---|
| `/etc/synofurb/certs/<id>/{fullchain.pem,privkey.pem,meta.json}` | the store; `<id>` is the first name (`*.x.org` becomes `_wildcard.x.org`, which no DNS name can clash with) | dirs 0700, files 0600, root |
| `/etc/synofurb/edge/settings.json` | e-mail, accepted terms (time), default environment, key type, resolver and service options | 0600 |
| `/etc/synofurb/edge/addr-tools.json` | **the challenge secret** (the only file that holds it), the service URL, an optional CA file (tests) | 0600 |
| `/etc/synofurb/edge/proxy.json` | front on/off, hostnames and certificates per target, the previous listener | 0600 |
| `/etc/synofurb/edge/haproxy.cfg.original` | the `haproxy.cfg` that was on the unit before the front first replaced it (kept once, only when it was not generated by us; the Debian default or an administrator's own file) | 0600 |
| `/etc/synofurb/lego/<production\|staging>/` | lego's `--path`: one account per Let's Encrypt environment, lego's own copies of the files | 0700 |
| `/usr/local/sbin/synofurb-acme-addr-tools` | generated lego `exec` adapter (Python 3, no secret inside, reads `addr-tools.json`) | 0700 |
| `/etc/haproxy/certs/<id>.pem`, `_default.pem` | chain then key for HAProxy; `_default.pem` is the management plane's own self-signed certificate | 0600 |
| `/etc/haproxy/haproxy.cfg`, `/etc/haproxy/synofurb-crt-list.txt` | generated (first line says so) | 0644 / 0600 |
| `/etc/systemd/system/synofurb-certs-renew.{service,timer}` | generated renewal pair | 0644 |
| `/var/lib/synofurb-mgmt/edge/` | `jobs.json`, `history.json` (rate-limit memory, 30 days), `renew.json` (last run), `pending.json` + `snap/` (an unconfirmed front change), `reverted.json` | 0600 / 0700 |

### Names, secret, DNS

* Every name goes through `valid_domain`: lowercase, labels `[a-z0-9-]` (non-ASCII labels become punycode), at least two labels, one leading `*.` only for DNS-01, no IP address, no private suffix (`.local`, `.lan`, `.internal`, ...), at most 10 names; nothing else
  reaches an argv, a file name or the HAProxy configuration. The certificate directory name is validated by `CERT_ID_RE` again on every use.
* The challenge secret: 32+ random URL-safe characters by default (`secrets.token_urlsafe(32)`), or the person's own (16..128 characters). It is **shown once**, when it is created, and then only as its target: `<sha224 hex, 56 lowercase characters>.challenges.addr.tools`
  (`hashlib.sha224`, tested against the FIPS 180-4 vectors). One target serves every name: the person creates `_acme-challenge.<name>  CNAME  <target>` for each name (a wildcard and its base share the record). Settings and the wizard show the target with a Copy button.
* **Check DNS** looks up `_acme-challenge.<name>` (CNAME) with `dig +short` when installed, otherwise with a small built-in UDP resolver (system resolvers from `/etc/resolv.conf`), and says `ok`, `wrong` (what it points at), `missing` or `error`. **Check service** POSTs only `secret` to the service and compares the sub-domain
  it answers with our own SHA-224 (nothing is published by that request). An issue with DNS-01 refuses to start while a record is `missing` or `wrong` (it would only burn a rate limit); `skip_dns_check` overrides, an `error` lookup only warns.
* The secret is never in an argv, an environment, a log line, the job log (every output line is scrubbed), the audit log (`secret` keys are redacted by the existing `redact`), an API answer after creation or the generated adapter.

### lego (all three challenges)

`lego_argv()` is the one place that builds the command line (the comment there says: verify on the unit with `lego --help`):

```
lego --server <acme url> --email <e-mail> --accept-tos --path /etc/synofurb/lego/<environment> --key-type ec256 --domains <name> [--domains <name> ...]
     --dns exec [--dns.disable-cp] [--dns.resolvers <ip[:port]> ...]      DNS-01   (env EXEC_PATH=<adapter>, EXEC_PROPAGATION_TIMEOUT=300, EXEC_POLLING_INTERVAL=5, EXEC_SEQUENCE_INTERVAL=10)
     --http --http.port 127.0.0.1:5080 | :80                              HTTP-01  (behind the front | directly)
     --tls  --tls.port  127.0.0.1:5443 | :443                              TLS-ALPN-01
     run                                                                   first issue and changes
     renew --days 30                                                       the timer (36500 for "Renew now"); falls back to `run` when lego has no files yet
```

The adapter implements `present [--] <FQDN> <value>` (POST `secret` + `txt` as a form to `https://challenges.addr.tools`, success is HTTP 201, three tries on a network error or 5xx), `cleanup` (nothing) and `timeout` (nothing). It accepts lego's calling convention with or without the `--`.
lego is installed on demand: `apt-get update`, then `apt-get install -y --no-install-recommends lego` (an offline unit gets "needs the Debian mirror and Internet access"); HAProxy the same way when the front is enabled. The terms of service are accepted explicitly in Settings and the time is stored; nothing runs before that.

### Issue, change, renew, remove

* Every operation is a **job** (steps, progress, log; `GET /api/certs/job/<id>`, kept in `jobs.json`), takes the shared `job.lock` (so it never runs together with an update of the management plane or an add-on job: HTTP 409 / CLI exit 1) and writes the audit log (`certs.job.start`, `certs.job.<kind>`).
* Steps: check (settings, DNS or ports, rate limits), tools, challenge, obtain (lego), verify (the new file is parsed by a small DER reader: the names must be in the SANs and the certificate must be valid), store, publish (combined PEM; HAProxy reload only when the front serves that certificate), done.
* **Staging** is a per-request switch (default from Settings); the two environments have separate lego accounts and folders.
* **Rate limits**: before calling Let's Encrypt (production only) the module refuses a 6th identical certificate within 7 days and a 6th failed attempt within an hour (kept in `history.json`), warns at 4 of 5, and turns lego's own output into plain messages (rate limit, NXDOMAIN, CAA, port in use, could not connect, DNS record did not appear). A failed first issue leaves a "Failed" row with its reason; a failed Change or renewal keeps the working certificate.
* **Pre-checks** for HTTP-01 / TLS-ALPN-01 (`/api/certs/precheck`): where each name resolves (this unit / another address / a private address that needs a port forward), whether port 80 or 443 is free or answered by the front. Reachability from the Internet **cannot** be tested from the unit and the dialog says so.
* **Remove** needs the typed name and is refused while a hostname uses the certificate.

### Renewal, health

`certs renew` (CLI, run by `synofurb-certs-renew.service`) renews every certificate with 30 days or fewer left (also when `last_error` is set, but not within 6 hours of a failure; `--force` ignores both), in one job under one lock, then reloads HAProxy **only if a certificate the front serves changed**:
the new PEM is written, `haproxy -c` runs, and the old PEM is put back if it fails. The timer is `OnCalendar=*-*-* 03,15:00:00`, `RandomizedDelaySec=3h`, `Persistent=true`; it is written and enabled after the first successful issue (idempotent). The last run is `renew.json` and shows on the page.
`health --json` gains an optional `certs` section only when certificates exist (`count`, `min_days`, `expiring[]`, `failed[]`); the schema number and every other key are unchanged. Warn at 14 days or fewer, critical at 3 days or fewer or expired, warn for a failed renewal (30 days or fewer left) or a failed first issue.

### The HAProxy front

Default **off**. `proxy enable` installs HAProxy if needed, builds the configuration, checks it, applies it and moves the management plane behind it:

```
:80    fe_http   HTTP-01 (/.well-known/acme-challenge/) -> lego 127.0.0.1:5080; everything else 301 -> https
:443   fe_tls    TCP: ALPN acme-tls/1 -> lego 127.0.0.1:5443 (TLS-ALPN-01); everything else -> 127.0.0.1:8444 with PROXY protocol
:8444  fe_https  (loopback only) TLS terminator: SNI picks the certificate (crt-list, default = the management plane's self-signed one),
                 the Host header picks the backend: be_mgmt (its hostname) / be_addon_<id> (the hostname assigned to that add-on's `web`), default_backend be_mgmt
```

* **Why a TCP front plus a loopback terminator**: ALPN routing of TLS-ALPN-01 needs the TCP layer, certificate selection and Host routing need the HTTP layer; one HAProxy frontend cannot be both. The Host header (not SNI) routes, because a browser may reuse one connection for several names of a wildcard certificate; unknown names and IP addresses reach the management plane, so the unit is always reachable.
  A loopback port is used instead of an abstract socket because abstract sockets cannot be re-bound on a seamless reload without extra setup. Consequence: any local user can connect to 127.0.0.1:8444 and (with a PROXY header) forge the client address the backends see.
* **Management plane behind the front**: it keeps its own TLS (self-signed, `127.0.0.1:<port>`, HAProxy re-encrypts with `verify none` on loopback), so cookies stay `__Host-` + Secure, HSTS and the Origin check (Host is passed through) are unchanged. `/etc/synofurb/mgmt.json` gets `bind: 127.0.0.1` and `trust_proxy: true` (the previous `bind` is remembered and restored);
  the service restarts 3 s after the job ends. With `trust_proxy`, `X-Forwarded-For` (which HAProxy overwrites, never appends) is believed **only from a loopback peer**, so login throttling and the audit log see the real client. Sessions are in memory: after the restart the person signs in again at the new address.
* **Add-ons**: each installed add-on with a `web` section can get a hostname and a certificate; the backend is `127.0.0.1:<web.port>` with `ssl verify none` when the scheme is https (per-add-on override http/https on the page). The add-on's own published port stays open on the LAN (HAProxy does not change containers); bind it to 127.0.0.1 in the manifest to hide it.
* **Safe apply** (the forgevault pattern): the candidate is rendered into a private temp directory and checked with `haproxy -c -f` (on failure nothing is touched: no write, no reload, the previous configuration stays); then the previous files are snapshotted, the new ones written and checked again at their real path, a **revert timer** is armed (`systemd-run --unit=synofurb-edge-revert --on-active=120s ... proxy revert --auto`, if that fails nothing is applied),
  HAProxy is started or reloaded, and the listening ports are checked. Any failure restores the snapshot (and the service state). The change stays **pending** until someone presses Keep (page banner on every page, `proxy confirm`) within two minutes; otherwise the timer reverts everything. A transient timer does not survive a reboot, so a pending change whose deadline passed is reverted when the web service starts.
  Applying again while a change is pending keeps the first snapshot (the last confirmed state).
* **Recovery path** (never locked out): the page and the help say it, and `python3 /opt/synofurb-mgmt/synofurb_mgmt.py proxy disable` run from the console or SSH restores the previous direct listener (`bind` back, `trust_proxy` removed, service restarted, HAProxy stopped by a transient timer 8 s later because restarting the web service kills the process that would stop it). Certificates, hostnames and the generated files stay. `proxy revert` goes back to the last confirmed state.
* Ports 80, 443 and 8444 must be free when enabling (checked, with the name of the port in the message). IPv6 is bound with `v6only` next to IPv4.

### CLI and API

```
synofurb_mgmt.py certs list|show ID|issue NAME... [--challenge dns-01|http-01|tls-alpn-01] [--environment staging|production] [--skip-dns-check]|change ID NAME...|renew [--id ID] [--force]|remove ID --confirm ID|check-dns NAME...
                  |settings [--email E] [--accept-tos] [--environment E] [--key-type T] [--generate-secret|--secret-stdin] [--check-secret] [--dns-skip-check on|off] [--dns-resolvers LIST] [--json]
synofurb_mgmt.py proxy status|enable|apply [--seconds N]|disable|test|confirm|revert [--auto]|assign TARGET HOSTNAME CERT|unassign TARGET        (TARGET is mgmt or an add-on id)
GET  /api/certs  /api/certs/<id>  /api/certs/job/<id>      POST /api/certs/settings|issue|precheck|check-dns|check-secret  /api/certs/<id>/renew|change|remove
GET  /api/proxy                                            POST /api/proxy/hostnames|test|enable|apply|disable|confirm|revert
```

Jobs answer 202 `{ok, job}`; errors are 400 (message safe to show), 404, 409 (busy). Session cookie, CSRF token and the same-origin check apply as everywhere. `--seconds` of `proxy enable|apply` sets the revert window. Exit codes: 0 ok, 1 failed or busy, 2 refused or bad input.

### Web UI

**Certificates** (a table like Add-ons: name with SANs and a challenge Tag, expiry with days left, state Tag, Renew now / Details / Change / Remove; the reason of a failure is text under the name) with the issue wizard (names, challenge with an info button each, production or staging, the CNAME with Copy and Check DNS, or what must be reachable with a pre-check;
the Issue button is `aria-disabled` with a visible reason), Settings (e-mail, terms, defaults, secret) and a job dialog. **Hostnames** (the management plane and every installed add-on with a web page: hostname, certificate, backend scheme; Check, Enable front / Apply / Disable front each behind a confirmation that says what changes and shows the recovery command; the pending banner with Keep and Revert).
Word budgets, info buttons instead of paragraphs, reasons as visible text, typed confirmation for Remove, Lucide icons only (`globe` was added), phone width and dark mode follow the web design rules and are checked by the audit (`tests/ui_edge.js`, `tests/ui_audit.js`). Screenshots: `design/screenshots/mgmt-demo-certs-*` and `mgmt-demo-proxy-*` (`?certs=states|unset|empty`, `?proxy=on|pending` in the demo).

### What was verified (in the sandbox, with doubles)

`apps/mgmt-plane/tests/test_edge.py`: strict names and traversal/injection attempts; SHA-224 vectors and the target shape; the DER reader against `openssl`; settings and file permissions; the adapter against a loopback HTTPS server that checks the form fields and answers 201 / 403 / 503; DNS through a fake `dig` and through the built-in resolver against a loopback UDP server (compressed answers);
the exact lego argv and environment recorded by a fake `lego` (and that no secret is in them); issue, staging/production folders, wildcard, change, failed change, rate limits, pre-checks, apt on demand and offline, job lock; renewal thresholds, back-off, forced renewal, reload only when needed, bad reload keeps the old file, the timer text; health thresholds and the unchanged schema; `haproxy.cfg` golden tests (minimal; several certificates, add-ons, a wildcard),
SNI/ALPN routing text, template-injection attempts; apply, `haproxy -c` failure (no reload, previous config kept), reload failure, start failure, missing ports, unarmed timer, busy ports, confirm, revert, a second apply keeping the first snapshot, startup recovery, disable; the API through the real HTTP server (auth, CSRF, origin, 400/404/409, audit without the secret, `X-Forwarded-For` trust); the CLI; the bundle contract in every place; both pages in a real browser (desktop and phone, reasons, typed confirmation, banner).

### What is NOT verified

* **Nothing ran against the real Let's Encrypt, a real DNS zone, a real `lego` or a real `haproxy`.** The lego flags above are from memory of the 4.9.1 CLI (`--http.port`/`--tls.port` as `host:port`, `--dns.disable-cp`, `--dns.resolvers`, `renew --days`, the `EXEC_*` variables, the `<path>/certificates/<first name>.crt|.key` file names with `*` as `_`): check them with `lego --help` and `lego renew --help` on the unit.
  Whether lego's exec provider passes `--` before the arguments in 4.9.1 is not known: the adapter accepts both.
* challenges.addr.tools: the form fields, 201 and the sub-domain answer follow the service's page (read 2026-11-03); its propagation delay, record lifetime, limits and error bodies are undocumented. lego's propagation check follows the CNAME into the service's zone by default (not confirmed for 4.9.1); `--dns-skip-check on` passes `--dns.disable-cp` as an escape.
* `haproxy -c` on the real HAProxy 3.0: `req.ssl_hello_type`, `req.ssl_alpn`, the `crt-list` format, `accept-proxy` on a loopback bind, `bind :::80 v6only`, `send-proxy-v2` are written from the documentation; the fake only records that a check ran. A reload under real systemd, the real listening check, HTTP/2 and WebSocket through the front are untested.
* systemd: `systemd-run --on-active` placing the revert outside the service, the timer and service units, `Persistent=true` after a reboot, restarting the web service while a request is open.
* Real TLS-ALPN-01 through the TCP split, HTTP-01 through the redirect rule, a certificate swap seen by a browser, OCSP, and renewal across the real 30-day boundary (the tests move a fake clock).
* A second unit on the same LAN name, IPv6-only units, and a NAT'd unit's port forwards are outside what the pre-check can know.

### Security review of this part

No shell anywhere (`shell=False`, one place starts processes, a test greps the module). Domain names: strict regex, punycode, no `/`, `..`, whitespace, control characters or wildcards outside DNS-01; certificate ids from a second regex; file names are built only from validated ids; the lego argv, the HAProxy configuration (hostnames, ids, integer ports, paths) and the generated adapter and unit files re-validate their inputs where they are written, and tests try newline, `}`, `;`, `#`, `$()`, backtick and path-traversal payloads in every slot.
The secret lives in one 0600 file; the adapter holds none; job logs, audit and API answers are scrubbed or never carry it; the lego child gets a fixed small environment plus `EXEC_*`. Everything that changes HAProxy is checked first and has an automatic way back. Known limits: the front's loopback PROXY port (above); add-on ports stay open; a root-owned `settings.json` change by an administrator can point the service URL at another https host
(the adapter would then send the secret there: it is only editable by root or through an authenticated, CSRF-protected, audited call); no OCSP stapling, no mTLS, no other ACME CA.

