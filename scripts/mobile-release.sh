#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage:
  scripts/mobile-release.sh candidate X.Y.Z

candidate  Publish the next immutable mobile-vX.Y.Z-rc.N candidate tag at the
           exact current commit of block/buzz's remote main branch.
USAGE
  exit 2
}

fail() {
  echo "Error: $*" >&2
  exit 1
}

require_clean_semver() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || \
    fail "'$1' is not a mobile release version (expected X.Y.Z)"
}

require_clean_tree() {
  git diff --quiet && git diff --cached --quiet && \
    [[ -z "$(git status --short --untracked-files=normal)" ]] || \
    fail "working tree is dirty; commit or stash changes first"
}

require_canonical_origin() {
  local origin_url
  origin_url="$(git config --get remote.origin.url || true)"
  case "$origin_url" in
    git@github.com:block/buzz.git|https://github.com/block/buzz.git|https://github.com/block/buzz)
      ;;
    *)
      fail "origin must be the canonical block/buzz repository (found '${origin_url:-<missing>}')" ;;
  esac
}

remote_ref_oid() {
  local ref="$1" line
  line="$(git ls-remote --refs origin "$ref")"
  [[ -n "$line" && "$line" != *$'\n'* ]] || return 1
  printf '%s' "${line%%$'\t'*}"
}

remote_main_commit_sha() {
  local ref="refs/heads/main" advertised_oid fetched_oid commit
  advertised_oid="$(remote_ref_oid "$ref")" || return 1
  git fetch -q --no-tags origin "$ref"
  fetched_oid="$(git rev-parse --verify FETCH_HEAD)"
  [[ "$fetched_oid" == "$advertised_oid" ]] || \
    fail "origin/main moved while it was being resolved"
  commit="$(git rev-parse --verify 'FETCH_HEAD^{commit}')" || return 1
  [[ "$commit" == "$advertised_oid" ]] || \
    fail "origin/main did not resolve directly to a commit"
  printf '%s' "$commit"
}

command="${1:-}"
case "$command" in
  candidate)
    [[ "$#" -eq 2 ]] || usage
    version="$2"
    require_clean_semver "$version"
    require_clean_tree
    require_canonical_origin

    main_sha="$(remote_main_commit_sha)" || fail "origin/main does not exist"
    next=1
    while IFS=$'\t' read -r _ ref; do
      [[ "$ref" =~ ^refs/tags/mobile-v${version//./\.}-rc\.([1-9][0-9]*)$ ]] || continue
      number="${BASH_REMATCH[1]}"
      (( number >= next )) && next=$((number + 1))
    done < <(git ls-remote --refs --tags origin "refs/tags/mobile-v${version}-rc.*")

    tag="mobile-v${version}-rc.${next}"
    if git show-ref --verify --quiet "refs/tags/$tag"; then
      fail "local tag $tag already exists; remove or rename it before retrying"
    fi
    git -c tag.gpgSign=false tag -a -m "Buzz Mobile $version release candidate $next" \
      "$tag" "$main_sha"
    if ! git push origin "refs/tags/$tag"; then
      git tag -d "$tag" >/dev/null
      fail "could not publish $tag"
    fi
    git tag -d "$tag" >/dev/null
    printf 'Published %s at origin/main commit %s. Use this exact tag in Release Mobile.\n' \
      "$tag" "$main_sha"
    ;;

  *) usage ;;
esac
