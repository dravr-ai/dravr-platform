#!/usr/bin/env bash
# ABOUTME: Compile-free pre-push check for inline 3+ segment paths that clippy::absolute_paths denies
# ABOUTME: Exists because full clippy is a ~12 min CI job, so this lint otherwise costs a whole red cycle
#
# `absolute_paths = "deny"` (Cargo.toml) with the default 2-segment threshold
# rejects any inline qualified path of three or more segments —
# `std::fs::read_to_string`, `std::env::set_var`, `crate::foo::bar`. The fix is
# always the same: import it and use the bare name.
#
# The lint only runs inside the full-workspace clippy job, which takes ~12
# minutes and is the last thing to report. On 2026-08-07 that cost three
# separate red-then-fix-then-wait cycles for this one rule. Grepping the diff
# catches the same thing in under a second, so the feedback arrives before the
# push instead of half an hour after it.
#
# Deliberately narrow, and the narrowness is measured rather than assumed. The
# lint fires on paths rooted at `std`, `core`, `alloc` or `crate`; it does NOT
# fire on third-party roots. `messaging_routes_test.rs` carries
# `chrono::Utc::now` and `ed25519_dalek::SigningKey::from_bytes` on a green
# build, while `std::env::set_var` in the same file failed CI — so matching any
# 3-segment path would flag known-good code on its first run.
#
# It scans only Rust files changed against the base ref, and skips `use`
# declarations (where qualified paths belong), comments, and attributes. It
# cannot see through macros or string literals and does not try: a false
# positive here trains people to ignore the check, which is worse than a miss.
# Clippy in CI remains the authority.

set -uo pipefail

BASE_REF="${1:-origin/main}"

CHANGED=$(git diff --name-only "$BASE_REF" HEAD -- '*.rs' 2>/dev/null \
    || git diff --name-only HEAD~1 HEAD -- '*.rs' 2>/dev/null \
    || true)

[ -z "$CHANGED" ] && exit 0

HITS=$(
    for f in $CHANGED; do
        [ -f "$f" ] || continue
        awk '
            /^[[:space:]]*(\/\/|\/\*|\*)/            { next }   # comments and doc comments
            /^[[:space:]]*(pub[[:space:]]+)?use[[:space:]]/ { next }   # use declarations
            # Attributes, including the continuation lines of a multi-line one.
            # Skipping only the opening `#[` left the body of
            #   #[deprecated(
            #       note = "Use crate::a::b() instead"
            #   )]
            # exposed, and a path inside an attribute string is not code —
            # clippy::absolute_paths never fires there, so reporting it is a
            # false positive that blocks a push for a line nobody can fix.
            /^[[:space:]]*#\[/ {
                depth = gsub(/\[/, "[") - gsub(/\]/, "]")
                if (depth > 0) in_attr = 1
                next
            }
            in_attr {
                depth += gsub(/\[/, "[") - gsub(/\]/, "]")
                if (depth <= 0) in_attr = 0
                next
            }
            {
                line = $0
                while (match(line, /(std|core|alloc|crate)(::[A-Za-z_][A-Za-z0-9_]*){2,}/)) {
                    print FILENAME ":" FNR ": " substr(line, RSTART, RLENGTH)
                    line = substr(line, RSTART + RLENGTH)
                }
            }
        ' "$f"
    done
)

if [ -n "$HITS" ]; then
    echo "  ❌ inline paths of 3+ segments (clippy::absolute_paths = deny):"
    echo "$HITS" | sed 's/^/     /'
    echo ""
    echo "     Import them instead:  use std::fs;  ... fs::read_to_string(..)"
    echo "     This is denied workspace-wide in Cargo.toml and fails the full"
    echo "     clippy job, ~12 minutes into CI."
    exit 1
fi

echo "  ✅ no inline 3+ segment paths in changed Rust files"
exit 0
