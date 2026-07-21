#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$repo_root/scripts/mobile-release.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
remote="$tmp/remote.git"
work="$tmp/work"
operator="$tmp/operator"
canonical_origin="git@github.com:block/buzz.git"

run_release() {
  local repo="$1"
  shift
  (
    cd "$repo"
    git config "url.file://$remote.insteadOf" "$canonical_origin"
    git config protocol.file.allow always
    "$script" "$@"
  )
}

fail() {
  echo "$*" >&2
  exit 1
}

git init -q --bare "$remote"
git init -q "$work"
git -C "$work" config user.name test
git -C "$work" config user.email test@example.com
git -C "$work" remote add origin "$canonical_origin"
echo first > "$work/file"
git -C "$work" add file
git -C "$work" commit -qm first
git -C "$work" branch -M main
git -C "$work" -c "url.file://$remote.insteadOf=$canonical_origin" \
  -c protocol.file.allow=always push -q -u origin main

# Candidate publication must work from a stale operator clone and tag the exact
# current remote main commit, never the operator's checkout.
git -c "url.file://$remote.insteadOf=$canonical_origin" \
  -c protocol.file.allow=always clone -q "$canonical_origin" "$operator"
git -C "$operator" config user.name test
git -C "$operator" config user.email test@example.com
echo remote-only >> "$work/file"
git -C "$work" commit -qam remote-only
git -C "$work" -c "url.file://$remote.insteadOf=$canonical_origin" \
  -c protocol.file.allow=always push -q origin main
remote_main_sha="$(git --git-dir="$remote" rev-parse refs/heads/main)"
if git -C "$operator" cat-file -e "$remote_main_sha^{commit}" 2>/dev/null; then
  fail "stale-clone fixture already contains the remote-only commit"
fi
run_release "$operator" candidate 1.2.3
[[ "$(git --git-dir="$remote" rev-parse 'refs/tags/mobile-v1.2.3-rc.1^{commit}')" == \
   "$remote_main_sha" ]]
[[ "$(git --git-dir="$remote" cat-file -t refs/tags/mobile-v1.2.3-rc.1)" == tag ]]
if git -C "$operator" show-ref --verify --quiet refs/tags/mobile-v1.2.3-rc.1; then
  fail "successful candidate publication stranded a local tag"
fi

# Existing remote identities are immutable and candidate numbers increase
# monotonically. A later candidate for the same marketing version may point at
# a newer main commit, while the prior candidate never moves.
rc1_tag_oid="$(git --git-dir="$remote" rev-parse refs/tags/mobile-v1.2.3-rc.1)"
echo newer >> "$work/file"
git -C "$work" commit -qam newer
git -C "$work" -c "url.file://$remote.insteadOf=$canonical_origin" \
  -c protocol.file.allow=always push -q origin main
new_main_sha="$(git --git-dir="$remote" rev-parse refs/heads/main)"
run_release "$operator" candidate 1.2.3
[[ "$(git --git-dir="$remote" rev-parse refs/tags/mobile-v1.2.3-rc.1)" == "$rc1_tag_oid" ]]
[[ "$(git --git-dir="$remote" rev-parse 'refs/tags/mobile-v1.2.3-rc.2^{commit}')" == \
   "$new_main_sha" ]]
[[ "$(git --git-dir="$remote" cat-file -t refs/tags/mobile-v1.2.3-rc.2)" == tag ]]

# Sequence from the highest exact remote RC even if there are gaps, and ignore
# malformed or other-version tags.
git -C "$work" -c tag.gpgSign=false tag -a -m gap mobile-v1.2.3-rc.7 "$new_main_sha"
git -C "$work" -c tag.gpgSign=false tag -a -m malformed mobile-v1.2.3-rc.08 "$new_main_sha"
git -C "$work" -c tag.gpgSign=false tag -a -m other mobile-v1.2.4-rc.99 "$new_main_sha"
git -C "$work" -c "url.file://$remote.insteadOf=$canonical_origin" \
  -c protocol.file.allow=always push -q origin \
  refs/tags/mobile-v1.2.3-rc.7 refs/tags/mobile-v1.2.3-rc.08 \
  refs/tags/mobile-v1.2.4-rc.99
run_release "$operator" candidate 1.2.3
[[ "$(git --git-dir="$remote" rev-parse 'refs/tags/mobile-v1.2.3-rc.8^{commit}')" == \
   "$new_main_sha" ]]

# A rejected publication must remove its temporary local tag and leave no
# remote identity behind.
failing_operator="$tmp/failing-operator"
git -c "url.file://$remote.insteadOf=$canonical_origin" \
  -c protocol.file.allow=always clone -q "$canonical_origin" "$failing_operator"
git -C "$failing_operator" config user.name test
git -C "$failing_operator" config user.email test@example.com
mkdir -p "$failing_operator/.git/hooks"
cat > "$failing_operator/.git/hooks/pre-push" <<'HOOK'
#!/usr/bin/env bash
exit 1
HOOK
chmod +x "$failing_operator/.git/hooks/pre-push"
git -C "$failing_operator" config core.hooksPath .git/hooks
if run_release "$failing_operator" candidate 9.9.9 >/dev/null 2>&1; then
  fail "candidate succeeded despite a rejected push"
fi
if git -C "$failing_operator" show-ref --verify --quiet refs/tags/mobile-v9.9.9-rc.1; then
  fail "failed candidate publication stranded a local tag"
fi
if git --git-dir="$remote" show-ref --verify --quiet refs/tags/mobile-v9.9.9-rc.1; then
  fail "failed candidate publication created a remote tag"
fi

# Publishing through a fork or an origin with no canonical identity is rejected.
fork_operator="$tmp/fork-operator"
git clone -q "$remote" "$fork_operator"
git -C "$fork_operator" config user.name test
git -C "$fork_operator" config user.email test@example.com
if (cd "$fork_operator" && "$script" candidate 2.0.0 >/dev/null 2>&1); then
  fail "noncanonical origin was accepted"
fi
if git --git-dir="$remote" show-ref --verify --quiet refs/tags/mobile-v2.0.0-rc.1; then
  fail "noncanonical origin published a candidate"
fi

# A colliding local tag is never overwritten or removed.
local_collision_oid="$(git -C "$operator" rev-parse HEAD)"
git -C "$operator" -c tag.gpgSign=false tag -a -m local-collision \
  mobile-v3.0.0-rc.1 "$local_collision_oid"
if run_release "$operator" candidate 3.0.0 >/dev/null 2>&1; then
  fail "colliding local tag was overwritten"
fi
[[ "$(git -C "$operator" rev-parse 'refs/tags/mobile-v3.0.0-rc.1^{commit}')" == \
   "$local_collision_oid" ]]
if git --git-dir="$remote" show-ref --verify --quiet refs/tags/mobile-v3.0.0-rc.1; then
  fail "colliding local tag was published"
fi

# Dirty trees and invalid marketing versions fail before publication.
echo dirty > "$operator/untracked"
if run_release "$operator" candidate 2.0.0 >/dev/null 2>&1; then
  fail "dirty operator tree was accepted"
fi
rm "$operator/untracked"
if run_release "$operator" candidate 1.2 >/dev/null 2>&1; then
  fail "invalid marketing version was accepted"
fi

# Mobile no longer has release branches, a finalization command, a stable alias,
# a GitHub Release call, or metadata-only release recipes.
if run_release "$operator" start 2.0.0 >/dev/null 2>&1; then
  fail "removed start command was accepted"
fi
if run_release "$operator" finalize 1.2.3-rc.2 >/dev/null 2>&1; then
  fail "removed finalize command was accepted"
fi
if git --git-dir="$remote" for-each-ref --format='%(refname)' refs/heads/mobile-release/ | grep -q .; then
  fail "mobile release branch was created"
fi
if git --git-dir="$remote" show-ref --verify --quiet refs/tags/mobile-v1.2.3; then
  fail "stable mobile tag alias was created"
fi
if grep -qE '(^|[^[:alnum:]_])(gh[[:space:]]+release|mobile-release/|finalize)([^[:alnum:]_]|$)' \
    "$script"; then
  fail "removed branch/finalization/GitHub Release behavior remains in script"
fi

grep -Fq 'version: 0.0.0+1' "$repo_root/mobile/pubspec.yaml"
if grep -qE 'release-mobile|bump-mobile-version|get-current-mobile-version' "$repo_root/Justfile"; then
  fail "metadata-only mobile release recipe remains in Justfile"
fi

echo "mobile release contract passed"
