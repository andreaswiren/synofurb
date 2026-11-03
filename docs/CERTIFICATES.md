# Certificates and the HAProxy front (Stage 2 design)

Status: **design**, not implemented. Written 2026-11-03 from the owner's requirements; implementation starts after the add-on framework (Stage 1, `docs/ADDONS.md`).

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

* Store: `/etc/synofurb/certs/<name>/{fullchain.pem,privkey.pem,meta.json}` (0600/0700, root); HAProxy gets combined PEMs in `/etc/haproxy/certs/` (one file per certificate; HAProxy serves the right one by SNI, `crt-list` or a certs directory).
* Several certificates: a certificate has a primary name, optional SANs (and a wildcard), a challenge type, the add-ons/hostnames that use it, status (valid until, last renewal, last error). The UI lists them like the add-ons page: cards, status Tags, actions Renew now / Change / Remove.
* HAProxy config is generated by the management plane (frontend 443 with SNI routing to the management plane and to each add-on's `web` backend on localhost; frontend 80 for HTTP-01 and redirect to https; TLS-ALPN-01 responder route), validated with `haproxy -c -f` before every reload and rolled back on failure (the same safe-apply pattern as forgevault's `lib/system/haproxy.ts`; check `/home/user/forgevault` read-only for reference, never modify it).
* Renewal: a systemd timer (twice a day, random delay) renews certificates within 30 days of expiry, reloads HAProxy only when a certificate changed, records the result, and the dashboard shows expiring/failed certificates. Staging server option for testing (Let's Encrypt staging) and rate-limit awareness (5 duplicate certificates per week).
* Add-on hostnames: each add-on's `web.hostname_hint` becomes a hostname the user maps to a certificate; the add-on keeps listening on localhost/its port and HAProxy terminates TLS (Frigate's own 8971 https can be re-encrypted or bypassed: per-add-on `backend_scheme`).

## Out of scope for the first cut

DNS providers with their own APIs (Cloudflare, ...), ACME accounts for other CAs, mTLS, OCSP stapling controls.
