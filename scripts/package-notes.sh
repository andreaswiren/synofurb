#!/usr/bin/env bash
# Release notes: the CHANGELOG.md section for the version if there is one, otherwise the git log since the previous tag,
# followed by a fixed "how to verify" block.
#
#   scripts/package-notes.sh --version 0.2.0 --out dist/RELEASE_NOTES.md [--tag v0.2.0] [--design-hash H] [--gate-status S] [--signed yes|no]
#
# CHANGELOG headings recognised: "## [0.2.0] - date", "## 0.2.0", "## v0.2.0" (Keep a Changelog style).
set -euo pipefail
# shellcheck source=./package-lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/package-lib.sh"

VERSION=""
TAG=""
OUT=""
HASH=""
GATE=""
SIGNED="no"
while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="${2:?}"; shift 2 ;;
        --tag) TAG="${2:?}"; shift 2 ;;
        --out) OUT="${2:?}"; shift 2 ;;
        --design-hash) HASH="${2:?}"; shift 2 ;;
        --gate-status) GATE="${2:?}"; shift 2 ;;
        --signed) SIGNED="${2:?}"; shift 2 ;;
        -h | --help) sed -n '2,8p' "$0"; exit 0 ;;
        *) pkg_die "unknown option: $1" ;;
    esac
done
if [ -z "$VERSION" ] || [ -z "$OUT" ]; then pkg_die "--version and --out are required"; fi
[ -n "$TAG" ] || TAG="v$VERSION"
mkdir -p "$(dirname "$OUT")"

section=""
if [ -f "$PKG_ROOT/CHANGELOG.md" ]; then
    section="$(awk -v v="$VERSION" '
        /^##[[:space:]]/ {
            h = $0; sub(/^##[[:space:]]+/, "", h); gsub(/[\[\]]/, "", h); sub(/^v/, "", h)
            split(h, parts, /[[:space:]]/)
            if (found) exit
            if (parts[1] == v) { found = 1; next }
        }
        found { print }
    ' "$PKG_ROOT/CHANGELOG.md" | sed -e :a -e '/^[[:space:]]*$/{$d;N;ba' -e '}' | sed '1{/^[[:space:]]*$/d;}')"
fi

source_note=""
if [ -z "$section" ]; then
    if git -C "$PKG_ROOT" rev-parse --git-dir >/dev/null 2>&1; then
        rev="HEAD"
        git -C "$PKG_ROOT" rev-parse -q --verify "refs/tags/$TAG" >/dev/null 2>&1 && rev="$TAG"
        prev="$(git -C "$PKG_ROOT" describe --tags --abbrev=0 --match 'v*' "$rev^" 2>/dev/null || true)"
        if [ -n "$prev" ]; then range="$prev..$rev"; else range="$rev"; fi
        section="$(git -C "$PKG_ROOT" log --no-merges --pretty='- %s (%h)' "$range" 2>/dev/null | head -n 200 || true)"
        since="all history"
        [ -z "$prev" ] || since="since $prev"
        source_note="_Generated from the git log ($since); no CHANGELOG.md section for $VERSION._"
    fi
    [ -n "$section" ] || section="_No changelog entry and no git history available for this release._"
fi

{
    printf '## synofurb %s\n\n' "$VERSION"
    printf '%s\n\n' "$section"
    [ -n "$source_note" ] && printf '%s\n\n' "$source_note"
    if [ "$GATE" = override ]; then
        printf '%s\n\n' '> **WARNING: the design approval gate was OVERRIDDEN by a maintainer for this release** (see design/approvals/OVERRIDE.md at the tag). The designs were not approved by the UX/UI critics.'
    fi
    cat <<NOTES
### Downloads

| Platform | Files |
|---|---|
| Windows (x86_64) | \`synofurb-$VERSION-windows-x86_64.msi\` (installer), \`.exe\` (NSIS setup), \`.zip\` (portable) |
| macOS (Apple silicon / Intel) | \`synofurb-$VERSION-macos-aarch64\` / \`-x86_64\` \`.dmg\`, \`.tar.gz\` (bare binary), \`.pkg\` when present |
| Linux (x86_64 / aarch64) | \`.deb\`, \`.rpm\`, \`.AppImage\`, \`.tar.gz\` (bare binary) |
| NAS side | \`synofurb-agent-<arch>-$VERSION.vmlinuz\`/\`.initrd\`, \`synofurb-dom\`, \`compat-models.json\`, \`synofurb-mgmt-*.tar.gz\` |

Quickest install: \`curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.sh | sh\` (Linux, macOS) or
\`irm https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.ps1 | iex\` (Windows). Details: docs/INSTALL.md.

### Verify your download

\`\`\`sh
minisign -Vm SHA256SUMS -P <public key from docs/RELEASING.md>    # authenticity of the checksum list
sha256sum --ignore-missing -c SHA256SUMS                          # integrity of the files you downloaded
\`\`\`
${HASH:+Design hash approved for this release: \`$HASH\`}
$([ "$SIGNED" = yes ] && printf 'SHA256SUMS is signed with minisign (SHA256SUMS.minisig).' || printf '**This is an unsigned build: SHA256SUMS.minisig is not attached.**')
NOTES
} >"$OUT"
pkg_log "wrote $OUT"
