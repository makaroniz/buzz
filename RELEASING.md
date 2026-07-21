# Releasing Buzz

Buzz has three independent release lanes. Desktop and relay use release PRs.
Mobile uses immutable release-candidate tags cut directly from remote `main`:

| Lane | Entry point | Artifact |
|------|-------------|----------|
| Desktop | `just release-desktop` | Signed desktop app (macOS/Linux) |
| Relay | `just release-relay` | `ghcr.io/block/buzz` container image |
| Mobile | `scripts/mobile-release.sh candidate X.Y.Z` | Exact `mobile-vX.Y.Z-rc.N` source identity |

The lanes version independently. Desktop reads its manifests, relay reads its
crate manifest, and mobile derives both source and marketing version from the
exact candidate tag. The mobile handoff to the private `buzz-releases` pipeline
remains manual because OSS CI cannot trigger private CI.

## Quick Start

```sh
# Desktop release (next patch version)
just release-desktop

# Desktop explicit version
just release-desktop 0.4.0

# Relay release
just release-relay
just release-relay 0.4.0

# Publish the next mobile candidate from the exact current remote main commit
scripts/mobile-release.sh candidate 0.5.0
```

Desktop and relay releases use metadata PRs. Mobile does not. Each
`mobile-vX.Y.Z-rc.N` tag is an immutable candidate and the artifact of record.
There is no mobile release branch, stable mobile tag alias, finalization step,
or mobile GitHub Release.

---

## How It Works

### Desktop

1. **`just release-desktop`** runs locally on `main`, creates or updates a
   `version-bump/<version>` PR, bumps the desktop manifests, regenerates
   lockfiles, and updates `CHANGELOG.md`.
2. **Merge the PR.** `auto-tag-on-release-pr-merge` pushes `v<version>`.
3. **The tag triggers `release.yml`.** It builds, signs, notarizes, and
   publishes the desktop app for macOS and Linux.

### Relay

1. **`just release-relay`** runs locally on `main`, creates or updates a
   `relay-release/<version>` PR, bumps `crates/buzz-relay/Cargo.toml`,
   regenerates `Cargo.lock`, and updates the relay changelog.
2. **Merge the PR.** `auto-tag-on-release-pr-merge` pushes
   `relay-v<version>`.
3. **The tag triggers `docker.yml`.** Stable releases update the version
   aliases and `latest`; prereleases do not.

Every push to `main` continues to publish the rolling relay `:main` and
`:sha-<7>` tags.

### Mobile

1. **Publish a candidate.** From a clean checkout whose `origin` is the
   canonical `block/buzz` repository, run
   `scripts/mobile-release.sh candidate X.Y.Z`. The script resolves and fetches
   the exact current `origin/main` commit, derives the next number from exact
   remote tags for that marketing version, and publishes an annotated
   `mobile-vX.Y.Z-rc.N` tag there. It never uses the operator's checked-out
   commit and never moves an existing candidate.
2. **Build the exact tag.** Enter the candidate tag as `mobile_ref` in the
   private Buzz mobile Buildkite pipeline. OSS CI deliberately cannot trigger
   that private pipeline. The tag supplies both source commit and release
   version. Flutter receives clean marketing version `X.Y.Z`; Buildkite's
   monotonically increasing build number supplies the platform build number.
3. **Promote tested artifacts.** Promote the already-built signed artifact for
   each platform through its store workflow. Record the exact tag with the
   build or rollout record. No source ref is changed and no final build is cut.

The iOS and Android artifacts for one marketing version may come from different
RC tags. For example, iOS can ship `mobile-v0.5.0-rc.2` while Android ships
`mobile-v0.5.0-rc.3`. Each platform's exact candidate tag is its source record.
There is intentionally no single selected or final candidate for the marketing
version.

`mobile/pubspec.yaml` keeps `0.0.0+1` only as a valid, visibly non-release
fallback for local development and validation builds. Release jobs always
inject both version fields. `mobile/CHANGELOG.md` is retained as historical
release data. It is not a release ledger for this flow.

---

## Version Sources

| Lane | Release version authority |
|------|---------------------------|
| Desktop | `desktop/package.json` and synchronized desktop manifests |
| Relay | `crates/buzz-relay/Cargo.toml` |
| Mobile | Exact `mobile-vX.Y.Z-rc.N` remote tag |

`just bump-desktop-version <version>` updates the desktop manifests and
regenerates their lockfiles. `just bump-relay-version <version>` updates the
relay crate and regenerates `Cargo.lock`. Mobile has no bump recipe or
release-metadata PR.

---

## Manual Fallback

Desktop supports the manual GitHub Actions fallback:

1. Go to **Actions > Release** in the GitHub UI.
2. Click **Run workflow**.
3. Provide the semver version and the immutable tag ref to build.

Mobile intentionally has no branch or arbitrary-ref fallback. The private
Buildkite pipeline accepts only an exact candidate tag.

---

## Internal Releases

For mobile, trigger the private
[Release Mobile pipeline](https://buildkite.com/runway/buzz-mobile-releases) with
an exact RC tag for the platform build being cut. For desktop, use
[Release Desktop](https://buildkite.com/runway/sprout-releases). See the
[buzz-releases README](https://github.com/squareup/buzz-releases#cutting-a-release)
for the private pipeline contract.

---

## What Gets Published

Desktop publishes two GitHub releases:

1. **`v<version>`**: the user-facing release with installers.
2. **`buzz-desktop-latest`**: the rolling auto-updater release.

Mobile publishes only annotated `mobile-vX.Y.Z-rc.N` git tags. Store artifacts
and rollout records retain the exact tag they used. Mobile does not publish a
GitHub Release or a stable `mobile-vX.Y.Z` alias.

---

## Platform Support

The release workflow builds **two separate macOS DMGs**: Apple
Silicon (`darwin-aarch64`, the `release` job) and Intel
(`darwin-x86_64`, the `release-macos-x64` job), plus Linux `.deb` and
`.AppImage`. Both macOS DMGs are codesigned, notarized, and attached to
the same `v<version>` release. Intel users download the `_x64.dmg`.

The Linux AppImage is post-processed by `desktop/scripts/fix-appimage.sh`,
which strips infra libraries over-bundled by linuxdeploy (they crash on
Mesa 25+ / GLib 2.88 distros; see
[tauri-apps/tauri#15665](https://github.com/tauri-apps/tauri/issues/15665))
and re-signs the artifact. As a result the AppImage relies on the
host's Wayland/GStreamer/graphics stack and requires GLib >= 2.72
(Ubuntu 22.04 or newer). The `release-linux` job builds inside a
`ubuntu:22.04` container for broad GLIBC compatibility.

---

## Prerequisites

- **Write access** to the `block/buzz` GitHub repository
- An `origin` remote whose configured URL is the canonical `block/buzz`
  repository
- Repository tag rules that make `mobile-v*` immutable while allowing the
  intended release operator to create a new candidate tag
- The following **GitHub Actions secrets** must be configured for the desktop
  release lane:

  | Secret | Purpose |
  |--------|---------|
  | `BUZZ_UPDATER_PUBLIC_KEY` | Tauri updater public key (minisign) |
  | `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater private key |
  | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for the private key |

The mobile candidate command uses `git` directly. It does not need the `gh` CLI,
GitHub workflow dispatch permissions, GitHub Releases permissions, or a mobile
release-branch ruleset.

---

## Troubleshooting

### `just release-desktop` fails with "must be on main branch"
Switch to `main` and pull latest before running the release recipe.

### `just release-desktop` fails with "working tree is dirty"
Commit or stash your changes before running the release recipe.

### New commits land after publishing a mobile candidate

Run `scripts/mobile-release.sh candidate <version>` again after the intended
fix reaches remote `main`. It publishes a new immutable RC tag at the new exact
remote commit. Continue referring to each tested or shipped platform artifact by
its own exact tag.

### A mobile candidate command selects the wrong RC number

Do not retry by moving or deleting a tag. Inspect the exact remote `mobile-v*`
tags and resolve the unexpected state. Candidate numbers are monotonically
increasing remote identities.

### A mobile candidate push is rejected by repository rules

Confirm the intended operator is allowed to create `mobile-v*` tags. Do not
weaken update or deletion protection. Existing candidate tags must remain
immutable.

### Auto-updater reports "no update available"
Verify that the `buzz-desktop-latest` release exists and contains a
valid `latest.json`. The manifest covers all four platform keys
(`darwin-aarch64`, `darwin-x86_64`, `linux-x86_64`,
`windows-x86_64`); a missing entry usually means that platform's
release job failed. Check the workflow run.
