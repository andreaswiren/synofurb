#!/usr/bin/env bash
# Tests for scripts/package-*.sh against a throw-away copy of the repo skeleton with a fake `synofurb` binary.
# Runs everywhere bash + python3 exist; rpm / cargo-packager / tomllib checks are skipped when unavailable.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/synofurb-pkg-tests.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
PASS=0; FAIL=0; SKIP=0; CURRENT=""; OUT=""; RC=0
ok() { PASS=$((PASS + 1)); printf '  ok   %s\n' "$CURRENT"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL %s: %s\n' "$CURRENT" "$*"; [ -n "$OUT" ] && printf '%s\n' "$OUT" | sed 's/^/       | /' | head -15; }
skip() { SKIP=$((SKIP + 1)); printf '  skip %s (%s)\n' "$CURRENT" "$*"; }
begin() { CURRENT="$1"; BAD=0; }
chk() { if [ "$2" -ne 0 ]; then bad "$1"; BAD=1; fi; }
finish() { [ "$BAD" = 0 ] && ok; }
run() { OUT="$("$@" 2>&1)"; RC=$?; }

# skeleton repo
SK="$WORK/repo"
mkdir -p "$SK/scripts" "$SK/packaging" "$SK/apps/mgmt-plane"
cp "$REPO"/scripts/package-*.sh "$SK/scripts/"
cp -R "$REPO/packaging/." "$SK/packaging/"
rm -f "$SK/packaging/.Packager.generated.toml"
cp "$REPO/LICENSE" "$SK/LICENSE"
printf '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.7.1"\nedition = "2024"\n\n[profile.release]\nstrip = true\n' >"$SK/Cargo.toml"
printf '0.3.0\n' >"$SK/apps/mgmt-plane/VERSION"
for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu aarch64-apple-darwin; do
    mkdir -p "$SK/target/$t/release"; cp /bin/true "$SK/target/$t/release/synofurb"; cp /bin/true "$SK/target/$t/release/synofurb-cli"
done
mkdir -p "$SK/target/x86_64-pc-windows-msvc/release"; printf 'MZ fake' >"$SK/target/x86_64-pc-windows-msvc/release/synofurb.exe"
printf 'MZ fake cli' >"$SK/target/x86_64-pc-windows-msvc/release/synofurb-cli.exe"
S="$SK/scripts"

echo "package scripts"
begin "portable tar.gz (Linux): name, contents, modes, reproducible"
run "$S/package-portable.sh" --target x86_64-unknown-linux-gnu --out "$WORK/o1"; chk "exit $RC" "$([ "$RC" = 0 ]; echo $?)"
F="$WORK/o1/synofurb-0.7.1-linux-x86_64.tar.gz"
chk "asset missing" "$([ -f "$F" ]; echo $?)"
LIST="$(tar -tvzf "$F" 2>/dev/null)"
for n in synofurb LICENSE README.txt synofurb.png; do chk "no $n in archive" "$(printf '%s' "$LIST" | grep -q " $n\$"; echo $?)"; done
chk "binary not 0755" "$(printf '%s' "$LIST" | grep -q '^-rwxr-xr-x.* synofurb$'; echo $?)"
chk "no top-level dir expected" "$(tar -tzf "$F" | grep -q /; test $? -ne 0; echo $?)"
a="$(sha256sum "$F" | cut -d' ' -f1)"; run "$S/package-portable.sh" --target x86_64-unknown-linux-gnu --out "$WORK/o1b"; b="$(sha256sum "$WORK/o1b/synofurb-0.7.1-linux-x86_64.tar.gz" | cut -d' ' -f1)"
chk "not reproducible" "$([ "$a" = "$b" ]; echo $?)"
finish

begin "portable zip (Windows) and macOS tar.gz"
run "$S/package-portable.sh" --target x86_64-pc-windows-msvc --out "$WORK/o2"; chk "exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "zip listing" "$(python3 -c "import zipfile,sys; n=zipfile.ZipFile(sys.argv[1]).namelist(); sys.exit(0 if n[0]=='synofurb.exe' and 'LICENSE' in n else 1)" "$WORK/o2/synofurb-0.7.1-windows-x86_64.zip"; echo $?)"
run "$S/package-portable.sh" --target aarch64-apple-darwin --out "$WORK/o2"; chk "exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "mac archive" "$(tar -tzf "$WORK/o2/synofurb-0.7.1-macos-aarch64.tar.gz" | grep -qx synofurb; echo $?)"
chk "mac archive must not contain the linux icon" "$(tar -tzf "$WORK/o2/synofurb-0.7.1-macos-aarch64.tar.gz" | grep -q png; test $? -ne 0; echo $?)"
run "$S/package-portable.sh" --target riscv64-unknown-linux-gnu --out "$WORK/o2"; chk "unsupported triple must fail" "$([ "$RC" != 0 ]; echo $?)"
run "$S/package-portable.sh" --target i686-pc-windows-msvc --out "$WORK/o2"; chk "unsupported triple must fail" "$([ "$RC" != 0 ]; echo $?)"
finish

begin "cli archive (Linux/macOS tar.gz, Windows zip): name, flat contents, modes, reproducible, no desktop files"
run "$S/package-cli.sh" --target x86_64-unknown-linux-gnu --out "$WORK/oc1"; chk "exit $RC" "$([ "$RC" = 0 ]; echo $?)"
F="$WORK/oc1/synofurb-cli-0.7.1-linux-x86_64.tar.gz"
chk "asset missing" "$([ -f "$F" ]; echo $?)"
LIST="$(tar -tvzf "$F" 2>/dev/null)"
for n in synofurb-cli LICENSE README.txt; do chk "no $n in archive" "$(printf '%s' "$LIST" | grep -q " $n\$"; echo $?)"; done
chk "binary not 0755" "$(printf '%s' "$LIST" | grep -q '^-rwxr-xr-x.* synofurb-cli$'; echo $?)"
chk "the CLI archive must not carry the desktop binary or icon" "$(printf '%s' "$LIST" | grep -Eq ' (synofurb|synofurb.png)$'; test $? -ne 0; echo $?)"
chk "no top-level dir expected" "$(tar -tzf "$F" | grep -q /; test $? -ne 0; echo $?)"
a="$(sha256sum "$F" | cut -d' ' -f1)"; run "$S/package-cli.sh" --target x86_64-unknown-linux-gnu --out "$WORK/oc1b"; b="$(sha256sum "$WORK/oc1b/synofurb-cli-0.7.1-linux-x86_64.tar.gz" | cut -d' ' -f1)"
chk "not reproducible" "$([ "$a" = "$b" ]; echo $?)"
run "$S/package-cli.sh" --target aarch64-unknown-linux-gnu --out "$WORK/oc2"; chk "aarch64 exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "aarch64 asset" "$([ -f "$WORK/oc2/synofurb-cli-0.7.1-linux-aarch64.tar.gz" ]; echo $?)"
run "$S/package-cli.sh" --target aarch64-apple-darwin --out "$WORK/oc2"; chk "mac exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "mac archive" "$(tar -tzf "$WORK/oc2/synofurb-cli-0.7.1-macos-aarch64.tar.gz" | grep -qx synofurb-cli; echo $?)"
run "$S/package-cli.sh" --target x86_64-pc-windows-msvc --out "$WORK/oc2"; chk "windows exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "zip listing" "$(python3 -c "import zipfile,sys; n=zipfile.ZipFile(sys.argv[1]).namelist(); sys.exit(0 if n[0]=='synofurb-cli.exe' and 'LICENSE' in n and 'synofurb.exe' not in n else 1)" "$WORK/oc2/synofurb-cli-0.7.1-windows-x86_64.zip"; echo $?)"
run "$S/package-cli.sh" --target riscv64-unknown-linux-gnu --out "$WORK/oc2"; chk "unsupported triple must fail" "$([ "$RC" != 0 ]; echo $?)"
run "$S/package-cli.sh" --target x86_64-unknown-linux-gnu --bin "$WORK/nope" --out "$WORK/oc2"; chk "missing binary must fail" "$([ "$RC" != 0 ]; echo $?)"; chk "tells how to build it" "$(printf '%s' "$OUT" | grep -q -- '--no-default-features --bin synofurb-cli'; echo $?)"
finish

begin "bundled kexec: in the CLI zip, the portable zip and the packager resources when the file exists, in none when it does not"
mkdir -p "$SK/assets/kexec/amd64"; head -c 150000 /dev/zero | tr '\0' 'K' >"$SK/assets/kexec/amd64/kexec"
run "$S/package-cli.sh" --target x86_64-pc-windows-msvc --out "$WORK/kx1"; chk "cli exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "cli zip has the kexec" "$(python3 -c "import zipfile,sys; z=zipfile.ZipFile(sys.argv[1]); i=z.getinfo('assets/kexec/amd64/kexec'); sys.exit(0 if i.file_size==150000 else 1)" "$WORK/kx1/synofurb-cli-0.7.1-windows-x86_64.zip"; echo $?)"
run "$S/package-portable.sh" --target x86_64-pc-windows-msvc --out "$WORK/kx2"; chk "portable exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "portable zip has the kexec" "$(python3 -c "import zipfile,sys; z=zipfile.ZipFile(sys.argv[1]); sys.exit(0 if 'assets/kexec/amd64/kexec' in z.namelist() and 'synofurb.exe' in z.namelist() else 1)" "$WORK/kx2/synofurb-0.7.1-windows-x86_64.zip"; echo $?)"
run "$S/package-cargo-packager.sh" --target x86_64-pc-windows-msvc --formats wix --render-only; chk "render exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "resources entry" "$(grep -q '^resources = \[{ src = "../assets/kexec/amd64/kexec", target = "assets/kexec/amd64/kexec" }\]' "$SK/packaging/.Packager.generated.toml"; echo $?)"
rm -rf "$SK/assets"
run "$S/package-cli.sh" --target x86_64-pc-windows-msvc --out "$WORK/kx3"; chk "cli without kexec exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "no kexec in the cli zip without the file" "$(python3 -c "import zipfile,sys; sys.exit(1 if any('kexec' in n for n in zipfile.ZipFile(sys.argv[1]).namelist()) else 0)" "$WORK/kx3/synofurb-cli-0.7.1-windows-x86_64.zip"; echo $?)"
run "$S/package-cargo-packager.sh" --target x86_64-pc-windows-msvc --formats wix --render-only; chk "render without exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "no resources entry without the file" "$(grep -q '^resources' "$SK/packaging/.Packager.generated.toml"; test $? -ne 0; echo $?)"
finish

begin "rpm: name, version, requires, files"
if ! command -v rpmbuild >/dev/null 2>&1 || ! command -v rpm >/dev/null 2>&1; then skip "rpmbuild not installed"; else
    run "$S/package-rpm.sh" --target x86_64-unknown-linux-gnu --out "$WORK/o3"; chk "exit $RC" "$([ "$RC" = 0 ]; echo $?)"
    R="$WORK/o3/synofurb-0.7.1-linux-x86_64.rpm"
    chk "rpm missing" "$([ -f "$R" ]; echo $?)"
    chk "version" "$(rpm -qp --qf '%{VERSION}' "$R" 2>/dev/null | grep -qx 0.7.1; echo $?)"
    chk "arch" "$(rpm -qp --qf '%{ARCH}' "$R" 2>/dev/null | grep -qx x86_64; echo $?)"
    chk "files" "$(rpm -qpl "$R" 2>/dev/null | grep -qx /usr/bin/synofurb; echo $?)"
    chk "requires vulkan-loader" "$(rpm -qpR "$R" 2>/dev/null | grep -qx vulkan-loader; echo $?)"
    # pre-release versions
    sed -i 's/^version = .*/version = "0.8.0-rc.1"/' "$SK/Cargo.toml"
    run "$S/package-rpm.sh" --target aarch64-unknown-linux-gnu --out "$WORK/o3"; chk "prerelease exit $RC" "$([ "$RC" = 0 ]; echo $?)"
    chk "prerelease rpm version" "$(rpm -qp --qf '%{VERSION}' "$WORK/o3/synofurb-0.8.0-rc.1-linux-aarch64.rpm" 2>/dev/null | grep -qx '0.8.0~rc.1'; echo $?)"
    sed -i 's/^version = .*/version = "0.7.1"/' "$SK/Cargo.toml"
    finish
fi

begin "cargo-packager config rendering: placeholders, signing hooks, valid TOML"
run "$S/package-cargo-packager.sh" --target x86_64-pc-windows-msvc --formats wix,nsis --render-only; chk "exit $RC" "$([ "$RC" = 0 ]; echo $?)"
G="$SK/packaging/.Packager.generated.toml"
chk "version" "$(grep -q '^version = "0.7.1"' "$G"; echo $?)"
chk "no placeholders left" "$(grep -q '@[A-Z_]*@' "$G"; test $? -ne 0; echo $?)"
chk "unsigned markers" "$(grep -q 'unsigned build: WINDOWS_CERT_THUMBPRINT' "$G" && grep -q 'unsigned build: APPLE_SIGNING_IDENTITY' "$G"; echo $?)"
OUT="$(WINDOWS_CERT_THUMBPRINT=0123456789ABCDEF0123456789ABCDEF01234567 APPLE_SIGNING_IDENTITY='Developer ID Application: Ann "A" Smith (ABCDE12345)' "$S/package-cargo-packager.sh" --target x86_64-pc-windows-msvc --formats wix --render-only 2>&1)"; RC=$?
chk "signed render exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "thumbprint rendered" "$(grep -q '^certificate-thumbprint = "0123456789ABCDEF0123456789ABCDEF01234567"' "$G"; echo $?)"
chk "identity escaped" "$(grep -qF 'signing-identity = "Developer ID Application: Ann \"A\" Smith (ABCDE12345)"' "$G"; echo $?)"
if python3 -c 'import tomllib' 2>/dev/null; then
    chk "generated config is valid TOML" "$(python3 -c 'import tomllib,sys; d=tomllib.load(open(sys.argv[1],"rb")); assert d["version"]=="0.7.1" and d["binaries"][0]["main"]' "$G" 2>/dev/null; echo $?)"
fi
OUT="$(WINDOWS_CERT_THUMBPRINT='x; rm -rf /' "$S/package-cargo-packager.sh" --target x86_64-pc-windows-msvc --formats wix --render-only 2>&1)"; RC=$?
chk "bad thumbprint rejected" "$([ "$RC" != 0 ]; echo $?)"
finish

begin "cargo-packager real run (deb) when cargo-packager is installed"
if ! command -v cargo-packager >/dev/null 2>&1; then skip "cargo-packager not installed"; else
    run "$S/package-cargo-packager.sh" --target x86_64-unknown-linux-gnu --formats deb --out "$WORK/o4"; chk "exit $RC" "$([ "$RC" = 0 ]; echo $?)"
    chk "renamed deb" "$([ -f "$WORK/o4/synofurb-0.7.1-linux-x86_64.deb" ]; echo $?)"
    finish
fi

begin "package-collect: contract validation and SHA256SUMS"
D="$WORK/rel"; mkdir -p "$D"
V=0.7.1; M=0.3.0
for f in windows-x86_64.msi windows-x86_64.exe windows-x86_64.zip macos-aarch64.dmg macos-aarch64.tar.gz macos-x86_64.dmg macos-x86_64.tar.gz \
    linux-x86_64.deb linux-x86_64.rpm linux-x86_64.AppImage linux-x86_64.tar.gz linux-aarch64.deb linux-aarch64.rpm linux-aarch64.AppImage linux-aarch64.tar.gz; do
    echo "$f" >"$D/synofurb-$V-$f"
done
for f in windows-x86_64.zip macos-aarch64.tar.gz macos-x86_64.tar.gz linux-x86_64.tar.gz linux-aarch64.tar.gz; do echo "cli $f" >"$D/synofurb-cli-$V-$f"; done
for a in amd64 arm64 armhf; do echo k >"$D/synofurb-agent-$a-$V.vmlinuz"; echo i >"$D/synofurb-agent-$a-$V.initrd"; done
echo d >"$D/synofurb-dom"; echo '{}' >"$D/compat-models.json"; echo m >"$D/synofurb-mgmt-$M.tar.gz"
run "$S/package-collect.sh" --dir "$D" --version $V --mgmt-version $M; chk "complete set exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "warns about optional" "$(printf '%s' "$OUT" | grep -q 'optional asset not present'; echo $?)"
chk "SHA256SUMS verifies" "$(cd "$D" && grep -v ' SHA256SUMS' SHA256SUMS | sha256sum -c --quiet - >/dev/null 2>&1; echo $?)"
chk "SHA256SUMS lists 30 files" "$([ "$(wc -l <"$D/SHA256SUMS" | tr -d ' ')" = 30 ]; echo $?)"
chk "sorted, two-space format" "$(grep -Eq '^[0-9a-f]{64}  synofurb-' "$D/SHA256SUMS"; echo $?)"
rm "$D/synofurb-cli-$V-linux-aarch64.tar.gz"
run "$S/package-collect.sh" --dir "$D" --version $V --mgmt-version $M; chk "missing CLI archive must fail" "$([ "$RC" != 0 ]; echo $?)"; chk "names the missing CLI archive" "$(printf '%s' "$OUT" | grep -q 'synofurb-cli-0.7.1-linux-aarch64.tar.gz'; echo $?)"
echo c >"$D/synofurb-cli-$V-linux-aarch64.tar.gz"; echo c >"$D/synofurb-cli-$V-windows-aarch64.zip"
run "$S/package-collect.sh" --dir "$D" --version $V --mgmt-version $M; chk "optional windows-aarch64 CLI zip is accepted: exit $RC" "$([ "$RC" = 0 ]; echo $?)"
rm "$D/synofurb-cli-$V-windows-aarch64.zip"
rm "$D/synofurb-$V-linux-aarch64.rpm"
run "$S/package-collect.sh" --dir "$D" --version $V --mgmt-version $M; chk "missing rpm must fail" "$([ "$RC" != 0 ]; echo $?)"; chk "names the missing file" "$(printf '%s' "$OUT" | grep -q 'synofurb-0.7.1-linux-aarch64.rpm'; echo $?)"
echo r >"$D/synofurb-$V-linux-aarch64.rpm"; echo x >"$D/synofurb-0.7.0-linux-x86_64.deb"
run "$S/package-collect.sh" --dir "$D" --version $V --mgmt-version $M; chk "stale/unexpected file must fail" "$([ "$RC" != 0 ]; echo $?)"; chk "names the unexpected file" "$(printf '%s' "$OUT" | grep -q 'UNEXPECTED file.*0.7.0'; echo $?)"
rm "$D/synofurb-0.7.0-linux-x86_64.deb"; : >"$D/synofurb-dom"
run "$S/package-collect.sh" --dir "$D" --version $V --mgmt-version $M; chk "empty asset must fail" "$([ "$RC" != 0 ]; echo $?)"
finish

begin "package-mgmt: flat tarball, version consistency guard, reproducible"
M="$SK/apps/mgmt-plane"
printf '#!/usr/bin/env python3\nVERSION = "0.3.0"\n' >"$M/synofurb_mgmt.py"; printf '[Unit]\n' >"$M/synofurb-mgmt.service"; printf '# mgmt\n' >"$M/README.md"
run "$S/package-mgmt.sh" --out "$WORK/o5"; chk "exit $RC" "$([ "$RC" = 0 ]; echo $?)"
T5="$WORK/o5/synofurb-mgmt-0.3.0.tar.gz"
chk "listing" "$([ "$(tar -tzf "$T5" | sort | tr '\n' ' ')" = "README.md VERSION synofurb-mgmt.service synofurb_mgmt.py " ]; echo $?)"
chk "script executable" "$(tar -tvzf "$T5" | grep -q '^-rwxr-xr-x.* synofurb_mgmt.py$'; echo $?)"
a="$(sha256sum "$T5" | cut -d' ' -f1)"; run "$S/package-mgmt.sh" --out "$WORK/o5b"; chk "reproducible" "$([ "$(sha256sum "$WORK/o5b/synofurb-mgmt-0.3.0.tar.gz" | cut -d' ' -f1)" = "$a" ]; echo $?)"
printf '#!/usr/bin/env python3\nVERSION = "1.0.0"\n' >"$M/synofurb_mgmt.py"
run "$S/package-mgmt.sh" --out "$WORK/o5c"; chk "mismatch must fail" "$([ "$RC" != 0 ]; echo $?)"; chk "explains" "$(printf '%s' "$OUT" | grep -q 'version mismatch'; echo $?)"
printf '#!/usr/bin/env python3\nDEFAULT_VERSION = "0.3.0"\nVERSION = read_version()\n' >"$M/synofurb_mgmt.py"
run "$S/package-mgmt.sh" --out "$WORK/o5d"; chk "DEFAULT_VERSION form accepted: $RC" "$([ "$RC" = 0 ]; echo $?)"
printf '#!/usr/bin/env python3\nDEFAULT_VERSION = "0.2.0"\n' >"$M/synofurb_mgmt.py"
run "$S/package-mgmt.sh" --out "$WORK/o5e"; chk "DEFAULT_VERSION mismatch must fail" "$([ "$RC" != 0 ]; echo $?)"
printf '#!/usr/bin/env python3\nx = 1\n' >"$M/synofurb_mgmt.py"
run "$S/package-mgmt.sh" --out "$WORK/o5f"; chk "no constant must fail" "$([ "$RC" != 0 ]; echo $?)"
finish

begin "package-agent: renames, verifies against the build's SHA256SUMS"
mkdir -p "$SK/boot/agent"
cat >"$SK/boot/agent/build.sh" <<'FB'
#!/usr/bin/env bash
# fake boot/agent/build.sh <arch> --version V --out DIR [--work DIR]
arch=$1; shift; out=""
while [ $# -gt 0 ]; do case $1 in --out) out=$2; shift 2;; *) shift 2;; esac; done
mkdir -p "$out"; echo "kernel $arch" >"$out/agent-$arch.vmlinuz"; echo "initrd $arch" >"$out/agent-$arch.initrd"
( cd "$out" && sha256sum agent-$arch.vmlinuz agent-$arch.initrd >> SHA256SUMS )
[ -n "${TAMPER:-}" ] && echo x >> "$out/agent-$arch.initrd"
exit 0
FB
chmod +x "$SK/boot/agent/build.sh"
run "$S/package-agent.sh" --arch armhf --out "$WORK/o6"; chk "exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "renamed" "$([ -f "$WORK/o6/synofurb-agent-armhf-0.7.1.vmlinuz" ] && [ -f "$WORK/o6/synofurb-agent-armhf-0.7.1.initrd" ]; echo $?)"
OUT="$(TAMPER=1 "$S/package-agent.sh" --arch arm64 --out "$WORK/o6b" 2>&1)"; RC=$?; chk "tampered build must fail" "$([ "$RC" != 0 ]; echo $?)"
run "$S/package-agent.sh" --arch riscv --out "$WORK/o6c"; chk "bad arch must fail" "$([ "$RC" != 0 ]; echo $?)"
finish

begin "package-notes: CHANGELOG section, git-log fallback, override warning, signed/unsigned"
git -C "$SK" init -q 2>/dev/null && git -C "$SK" -c user.name=t -c user.email=t@t commit -q --allow-empty -m "first" && git -C "$SK" tag v0.7.0 && git -C "$SK" -c user.name=t -c user.email=t@t commit -q --allow-empty -m "fix the thing" 
printf '# Changelog\n\n## [0.7.1] - 2026-11-01\n\n### Fixed\n- the NAS probe\n\n## [0.7.0] - 2026-10-01\n- old stuff\n' >"$SK/CHANGELOG.md"
run "$S/package-notes.sh" --version 0.7.1 --out "$WORK/n1.md" --signed yes --design-hash abc123 --gate-status approved; chk "exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "has the changelog entry" "$(grep -q 'the NAS probe' "$WORK/n1.md"; echo $?)"
chk "excludes the older entry" "$(grep -q 'old stuff' "$WORK/n1.md"; test $? -ne 0; echo $?)"
chk "signed text" "$(grep -q 'SHA256SUMS is signed' "$WORK/n1.md"; echo $?)"
chk "design hash" "$(grep -q 'abc123' "$WORK/n1.md"; echo $?)"
rm "$SK/CHANGELOG.md"
run "$S/package-notes.sh" --version 0.7.1 --out "$WORK/n2.md" --gate-status override; chk "exit $RC" "$([ "$RC" = 0 ]; echo $?)"
chk "git log fallback" "$(grep -q 'fix the thing' "$WORK/n2.md"; echo $?)"
chk "range starts at previous tag" "$(grep -q '(since v0.7.0)' "$WORK/n2.md"; echo $?)"
chk "override warning" "$(grep -q 'OVERRIDDEN' "$WORK/n2.md"; echo $?)"
chk "unsigned warning" "$(grep -q 'unsigned build' "$WORK/n2.md"; echo $?)"
finish

echo; printf 'passed: %s  failed: %s  skipped: %s\n' "$PASS" "$FAIL" "$SKIP"
[ "$FAIL" = 0 ]
