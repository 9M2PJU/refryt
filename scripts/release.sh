#!/usr/bin/env bash
# Cutting a release: bump the version, check what was built, publish it.
#
#   scripts/release.sh prep <version> [--dry-run] [--skip-gate] [--perf]
#       Sets the version (Cargo.toml, Cargo.lock) and the Flatpak metainfo release entry, runs the
#       gate (fmt, clippy, tests; with --perf also scripts/perf.sh), and commits "Release <version>".
#       Refuses a dirty tree, a tree carrying the private local patch, a version that
#       is not newer than the current one, or one whose tag already exists.
#
#   scripts/release.sh verify <version>
#       Checks that the repo and everything in dist/ agree on <version>: the version in Cargo.toml,
#       the metainfo, a consistent Cargo.lock, a clean tree, a tag (if there is one) on HEAD, and
#       that each artifact exists, is newer than HEAD's commit (so it was built from it) and has
#       the right contents. The macOS image is built on a Mac, so its absence is a warning.
#
#   scripts/release.sh publish <version> --notes FILE [--without-macos] [--yes]
#       Verifies, then tags v<version>, pushes main and the tag, and creates the GitHub release
#       with the artifacts attached. Prints what it would do unless --yes is given.
#
# Nothing here builds: run `just appimage`, `./scripts/build-windows.sh`, `just flatpak` (and
# `./scripts/build-macos.sh` on the Mac) after `prep` and before `verify`.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

die() { echo "error: $*" >&2; exit 1; }
warn() { echo "warning: $*" >&2; }

current_version() {
    cargo metadata --no-deps --offline --format-version 1 \
        | python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "refryt"))'
}

# Whether Cargo.lock agrees with the manifests. Resolved for this machine's platform only: the
# full resolve wants the crates for every platform downloaded, which an offline run does not have.
lock_consistent() {
    cargo metadata --locked --offline --format-version 1 \
        --filter-platform "$(rustc -vV | sed -n 's/^host: //p')" > /dev/null 2>&1
}

valid_version() { [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; }

newer_than() { # newer_than <a> <b>: a > b
    [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -1)" = "$1" ]
}

# Edits to tracked files make a build that is not the commit it claims to be.
need_clean_tree() {
    if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
        git status --short --untracked-files=no >&2
        die "tracked files have uncommitted changes; commit or stash them first"
    fi
}

# The private build's feature must never reach a release: the patch lives outside the repo and
# is applied only to a local tree, so a tree that has it is not a release tree.
private_patch_applied() {
    grep -rq "unlocked-downloads" --exclude=release.sh crates/refryt/Cargo.toml crates/refryt/src scripts 2>/dev/null
}

need_no_private_patch() {
    if private_patch_applied; then
        die "the private local patch is applied; remove it before a release"
    fi
}

need_branch_main() {
    local b
    b="$(git rev-parse --abbrev-ref HEAD)"
    [ "$b" = "main" ] || die "releases are cut from main, not '$b'"
}

artifacts() { # artifacts <version>: "<kind> <path>" lines
    local v="$1"
    echo "appimage dist/refryt-$v-x86_64.AppImage"
    echo "windows-zip dist/refryt-$v-windows-x86_64.zip"
    echo "windows-setup dist/refryt-$v-windows-x86_64-setup.exe"
    echo "flatpak dist/refryt-$v-x86_64.flatpak"
    echo "macos dist/refryt-$v-macos-arm64.dmg"
}

cmd_prep() {
    local v="${1:-}"; shift || true
    local dry=0 gate=1 perf=0
    for a in "$@"; do
        case "$a" in
            --dry-run) dry=1 ;;
            --skip-gate) gate=0 ;;
            --perf) perf=1 ;;
            *) die "unknown option $a" ;;
        esac
    done
    valid_version "$v" || die "usage: release.sh prep <major.minor.patch>"
    local cur
    cur="$(current_version)"
    newer_than "$v" "$cur" || die "$v is not newer than the current version $cur"
    need_clean_tree
    need_no_private_patch
    need_branch_main
    if git rev-parse -q --verify "refs/tags/v$v" > /dev/null; then
        die "tag v$v already exists"
    fi
    local today
    today="$(date +%F)"
    echo "release $cur -> $v ($today)"
    if [ "$dry" = 1 ]; then
        echo "dry run: would set the version in Cargo.toml and Cargo.lock, add the metainfo entry,"
        echo "         run the gate$([ "$perf" = 1 ] && echo ' and perf.sh'), and commit \"Release $v\""
        return 0
    fi

    python3 - "$cur" "$v" "$today" <<'EOF'
import re, sys
cur, new, today = sys.argv[1:4]
# Cargo.toml: the workspace package's version line, and only that.
p = "Cargo.toml"
s = open(p).read()
head, sep, tail = s.partition("[workspace.package]")
assert sep, "no [workspace.package] in Cargo.toml"
tail, n = re.subn(r'^version = "%s"$' % re.escape(cur), 'version = "%s"' % new, tail, count=1, flags=re.M)
assert n == 1, "the workspace version line was not found"
open(p, "w").write(head + sep + tail)
# Flatpak metainfo: the newest release goes first.
p = "packaging/flatpak/io.github.gridoe.refryt.metainfo.xml"
s = open(p).read()
entry = '<release version="%s" date="%s"/>' % (new, today)
if 'version="%s"' % new not in s:
    s = s.replace("<releases>\n", "<releases>\n    %s\n" % entry, 1)
    assert entry in s, "no <releases> block in the metainfo"
    open(p, "w").write(s)
EOF
    # The lock file records the workspace crates' versions; refresh just those.
    cargo update --workspace --offline > /dev/null
    [ "$(current_version)" = "$v" ] || die "the version did not change to $v"
    lock_consistent || die "Cargo.lock is not consistent with the new version"

    if [ "$gate" = 1 ]; then
        echo "== gate =="
        cargo fmt --all --check
        cargo clippy --workspace --all-targets -- -D warnings
        cargo clippy -p refryt-resolve --features native-resolver -- -D warnings
        cargo test --workspace
        [ "$perf" = 1 ] && ./scripts/perf.sh
    else
        warn "the gate was skipped"
    fi

    git add Cargo.toml Cargo.lock packaging/flatpak/io.github.gridoe.refryt.metainfo.xml
    git commit -q -m "Release $v"
    echo "committed: $(git log -1 --oneline)"
    echo
    echo "next: build the artifacts, then scripts/release.sh verify $v"
    echo "      just appimage && ./scripts/build-windows.sh && just flatpak   (and the macOS image on a Mac)"
}

cmd_verify() {
    local v="${1:-}"
    valid_version "$v" || die "usage: release.sh verify <major.minor.patch>"
    local errors=0 warnings=0
    bad() { echo "  FAIL  $*"; errors=$((errors + 1)); }
    ok() { echo "  ok    $*"; }
    meh() { echo "  warn  $*"; warnings=$((warnings + 1)); }

    echo "== repository =="
    [ "$(current_version)" = "$v" ] && ok "Cargo.toml is $v" || bad "Cargo.toml is $(current_version), not $v"
    grep -q "<release version=\"$v\"" packaging/flatpak/io.github.gridoe.refryt.metainfo.xml \
        && ok "the Flatpak metainfo lists $v" || bad "the Flatpak metainfo has no release entry for $v"
    lock_consistent && ok "Cargo.lock matches" || bad "Cargo.lock is out of step with Cargo.toml"
    if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
        bad "tracked files have uncommitted changes"
    else
        ok "the tree is clean"
    fi
    if private_patch_applied; then
        bad "the private local patch is applied"
    else
        ok "no private patch"
    fi
    local head_time
    head_time="$(git log -1 --format=%ct)"
    if git rev-parse -q --verify "refs/tags/v$v" > /dev/null; then
        [ "$(git rev-parse "v$v^{commit}")" = "$(git rev-parse HEAD)" ] \
            && ok "tag v$v is HEAD" || bad "tag v$v is not HEAD"
    else
        ok "tag v$v not made yet"
    fi

    echo "== artifacts =="
    local kind path
    while read -r kind path; do
        if [ ! -f "$path" ]; then
            if [ "$kind" = macos ]; then
                meh "$path is missing: build it on the Mac (scripts/build-macos.sh) and copy it into dist/"
            else
                bad "$path is missing"
            fi
            continue
        fi
        local size mtime
        size="$(du -h "$path" | cut -f1)"
        mtime="$(stat -c %Y "$path")"
        if [ "$mtime" -lt "$head_time" ]; then
            bad "$path ($size) is older than HEAD's commit: rebuild it"
            continue
        fi
        case "$kind" in
            windows-zip)
                local list
                list="$(unzip -l "$path" 2>/dev/null)" || { bad "$path is not a readable zip"; continue; }
                grep -q "refryt.exe" <<<"$list" && grep -q "libmpv-2.dll" <<<"$list" \
                    && ok "$path ($size): refryt.exe and libmpv-2.dll inside" \
                    || bad "$path ($size) is missing refryt.exe or libmpv-2.dll" ;;
            *) ok "$path ($size)" ;;
        esac
    done < <(artifacts "$v")

    echo
    echo "$errors failure(s), $warnings warning(s)"
    [ "$errors" -eq 0 ]
}

cmd_publish() {
    local v="${1:-}"; shift || true
    local notes="" yes=0 nomac=0
    while [ $# -gt 0 ]; do
        case "$1" in
            --notes) notes="${2:-}"; shift ;;
            --yes) yes=1 ;;
            --without-macos) nomac=1 ;;
            *) die "unknown option $1" ;;
        esac
        shift
    done
    valid_version "$v" || die "usage: release.sh publish <version> --notes FILE [--yes]"
    [ -f "$notes" ] || die "--notes needs a notes file"
    need_branch_main
    cmd_verify "$v" || die "verify failed; fix that first"
    local assets=() kind path
    while read -r kind path; do
        if [ -f "$path" ]; then
            assets+=("$path")
        elif [ "$kind" = macos ] && [ "$nomac" = 0 ]; then
            die "the macOS image is missing; build it on the Mac, or pass --without-macos"
        fi
    done < <(artifacts "$v")
    command -v gh > /dev/null || die "gh is needed to create the release"

    local tag_cmd=(git tag -a "v$v" -m "refryt $v")
    local push_cmd=(git push origin main "v$v")
    local release_cmd=(gh release create "v$v" "${assets[@]}" --title "refryt $v" --notes-file "$notes")
    if git rev-parse -q --verify "refs/tags/v$v" > /dev/null; then
        tag_cmd=(true)
    fi
    echo
    echo "== publish =="
    printf '  %q ' "${tag_cmd[@]}"; echo
    printf '  %q ' "${push_cmd[@]}"; echo
    printf '  %q ' "${release_cmd[@]}"; echo
    if [ "$yes" != 1 ]; then
        echo
        echo "dry run: nothing was tagged, pushed or published. Run again with --yes to do it."
        return 0
    fi
    "${tag_cmd[@]}"
    "${push_cmd[@]}"
    "${release_cmd[@]}"
}

case "${1:-}" in
    prep) shift; cmd_prep "$@" ;;
    verify) shift; cmd_verify "$@" ;;
    publish) shift; cmd_publish "$@" ;;
    *) sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'; exit 1 ;;
esac
