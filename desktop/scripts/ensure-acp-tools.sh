#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app_root="$(cd "$script_dir/.." && pwd)"
lock_file="${ACP_TOOLS_LOCK_FILE:-$app_root/acp-tools.lock.json}"

# shellcheck source=lib/acp-node-wrapper.sh
source "$script_dir/lib/acp-node-wrapper.sh"

usage() {
  cat <<'USAGE'
Usage: desktop/scripts/ensure-acp-tools.sh [--target <target-triple>] [--print-bin-dir]

Installs the ACP bridge tools pinned in acp-tools.lock.json into the shared
Buzz dev cache. The lockfile is target-specific; only entries matching the
requested target are prepared. Each tool is installed as a vendored npm
package tree with a small executable wrapper, validated against the locked
versions and integrity hashes. Installs are bridge-only: --omit=optional
skips the SDK/codex platform packages that vendor native CLIs, so the trees
stay pure JS and the desktop app points each bridge at the user's own
claude/codex CLI at spawn time. Unix targets get a bash wrapper shim;
Windows targets get the compiled buzz-acp-node-launcher staged as
<binary>.exe next to a <binary>.shim.json manifest (built with cargo,
override with ACP_NODE_LAUNCHER_EXE).

Environment variables:
  ACP_TOOLS_LOCK_FILE    lockfile path (default: desktop/acp-tools.lock.json)
  ACP_TOOLS_CACHE_DIR    cache dir override
USAGE
}

default_cache_root() {
  if [[ -n "${XDG_CACHE_HOME:-}" ]]; then
    printf '%s/buzz-dev/acp-tools\n' "$XDG_CACHE_HOME"
    return
  fi
  case "$(uname -s)" in
    Darwin) printf '%s/Library/Caches/buzz-dev/acp-tools\n' "$HOME" ;;
    *) printf '%s/.cache/buzz-dev/acp-tools\n' "$HOME" ;;
  esac
}

target=""
print_bin_dir=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      target="${2:-}"
      [[ -n "$target" ]] || { echo "--target requires a value" >&2; exit 1; }
      shift 2
      ;;
    --print-bin-dir)
      print_bin_dir=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "$target" ]]; then
  target="$(rustc -vV | sed -n 's|host: ||p')"
fi
if [[ -z "$target" ]]; then
  echo "Could not determine rust host target. Pass --target explicitly." >&2
  exit 1
fi

cache_root="${ACP_TOOLS_CACHE_DIR:-$(default_cache_root)}"
bin_dir="$cache_root/bin/$target"

if [[ ! -f "$lock_file" ]]; then
  echo "ACP tools lockfile not found: $lock_file" >&2
  exit 1
fi

require_tool() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Required tool missing: $1" >&2
    exit 1
  fi
}

require_tool node
require_tool npm

lock_entries="$(node - "$lock_file" "$target" <<'NODE'
const fs = require("node:fs");
const [lockFile, target] = process.argv.slice(2);
const data = JSON.parse(fs.readFileSync(lockFile, "utf8"));
const entries = (data.tools ?? []).filter((tool) => tool.target === target);
function requireString(entry, field) {
  if (typeof entry[field] !== "string" || entry[field].trim() === "") {
    throw new Error(`Invalid ACP tool lock entry for ${entry.id ?? "(unknown)"}: missing ${field}`);
  }
}
for (const entry of entries) {
  if (entry.source !== "npm") {
    throw new Error(`Invalid ACP tool lock entry for ${entry.id}: unsupported source ${entry.source}`);
  }
  for (const field of [
    "id",
    "binary",
    "target",
    "package",
    "version",
    "integrity",
    "tarball",
    "dependencyPackage",
    "dependencyVersion",
    "dependencyIntegrity",
    "dependencyTarball",
  ]) {
    requireString(entry, field);
  }
}
process.stdout.write(JSON.stringify(entries));
NODE
)"

entry_count="$(node -e 'process.stdout.write(String(JSON.parse(process.argv[1]).length))' "$lock_entries")"
mkdir -p "$bin_dir"
if [[ "$entry_count" == "0" ]]; then
  find "$bin_dir" -type f -delete
  # stderr so the notice shows up in release build logs even when stdout is
  # reserved for --print-bin-dir consumers (prepare-acp-tools-resource.sh).
  echo "No ACP tools locked for target $target." >&2
  if [[ "$print_bin_dir" == "1" ]]; then
    printf '%s\n' "$bin_dir"
  fi
  exit 0
fi

# Windows targets stage the compiled launcher shim instead of a bash
# wrapper; it needs the repo's Rust toolchain. Built once up front — the
# per-tool loop below runs in a pipeline subshell.
launcher_exe=""
if acp_target_is_windows "$target"; then
  require_tool cargo
  launcher_exe="$(acp_node_launcher_exe "$target")"
fi

validate_npm_install() {
  local install_dir="$1"
  local package="$2"
  local version="$3"
  local integrity="$4"
  local dependency_package="$5"
  local dependency_version="$6"
  local dependency_integrity="$7"
  local claude_code_version="$8"

  node - "$install_dir" "$package" "$version" "$integrity" "$dependency_package" "$dependency_version" "$dependency_integrity" "$claude_code_version" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");

const [
  installDir,
  packageName,
  expectedVersion,
  expectedIntegrity,
  dependencyPackageName,
  expectedDependencyVersion,
  expectedDependencyIntegrity,
  expectedClaudeCodeVersion,
] = process.argv.slice(2);

function packagePath(name, ...segments) {
  return path.join(installDir, "node_modules", ...name.split("/"), ...segments);
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function assertEqual(actual, expected, label) {
  if (actual !== expected) {
    throw new Error(`${label}: expected ${expected}, got ${actual}`);
  }
}

function packageLockEntry(lock, packageName) {
  const suffix = `node_modules/${packageName}`;
  const match = Object.entries(lock.packages ?? {}).find(([key]) => key === suffix || key.endsWith(`/${suffix}`));
  if (!match) {
    throw new Error(`package-lock entry not found for ${packageName}`);
  }
  return match[1];
}

const packageJson = readJson(packagePath(packageName, "package.json"));
assertEqual(packageJson.name, packageName, `${packageName} name`);
assertEqual(packageJson.version, expectedVersion, `${packageName} version`);

const lock = readJson(path.join(installDir, "package-lock.json"));
assertEqual(packageLockEntry(lock, packageName).integrity, expectedIntegrity, `${packageName} integrity`);

const dependencyPackageJson = readJson(packagePath(dependencyPackageName, "package.json"));
assertEqual(
  dependencyPackageJson.name,
  dependencyPackageName,
  `${dependencyPackageName} name`,
);
assertEqual(
  dependencyPackageJson.version,
  expectedDependencyVersion,
  `${dependencyPackageName} version`,
);
if (expectedClaudeCodeVersion && expectedClaudeCodeVersion !== "null") {
  assertEqual(
    dependencyPackageJson.claudeCodeVersion,
    expectedClaudeCodeVersion,
    `${dependencyPackageName} claudeCodeVersion`,
  );
}
assertEqual(
  packageLockEntry(lock, dependencyPackageName).integrity,
  expectedDependencyIntegrity,
  `${dependencyPackageName} integrity`,
);

// Bridge-only invariant: --omit=optional must have skipped the platform
// packages that vendor a native CLI. They install as scope siblings of the
// dependency package (`<dependencyPackage>-<platform>`), so any such
// directory means a native CLI slipped into the bundle.
const dependencySegments = dependencyPackageName.split("/");
const dependencyBasename = dependencySegments.at(-1);
const scopeDir = path.join(
  installDir,
  "node_modules",
  ...dependencySegments.slice(0, -1),
);
const vendoredPlatformPackages = fs
  .readdirSync(scopeDir)
  .filter((name) => name.startsWith(`${dependencyBasename}-`));
if (vendoredPlatformPackages.length > 0) {
  throw new Error(
    `bridge-only install unexpectedly vendored native CLI package(s): ${vendoredPlatformPackages.join(", ")}`,
  );
}
NODE
}

node -e '
const entries = JSON.parse(process.argv[1]);
for (const entry of entries) {
  console.log([
    entry.id,
    entry.binary,
    entry.package,
    entry.version,
    entry.integrity,
    entry.tarball,
    entry.nodeEngine ?? ">=22",
    entry.dependencyPackage,
    entry.dependencyVersion,
    entry.dependencyIntegrity,
    entry.dependencyTarball,
    entry.claudeCodeVersion ?? "",
  ].join("\x1f"));
}
' "$lock_entries" | while IFS=$'\x1f' read -r id binary package version integrity tarball node_engine dependency_package dependency_version dependency_integrity dependency_tarball claude_code_version; do
  [[ -n "$id" ]] || continue

  tool_dir="$cache_root/$target/$id/$version"
  install_dir="$tool_dir/npm"
  package_dir="$install_dir/node_modules/$package"
  entrypoint="$package_dir/dist/index.js"
  staged_bin="$bin_dir/$(acp_staged_binary_name "$binary" "$target")"
  # Windows shims embed a bin-dir-relative entrypoint: under Git Bash the
  # absolute cache path is POSIX-style (/c/Users/...), which the native
  # launcher cannot resolve.
  entrypoint_from_bin_dir="../../$target/$id/$version/npm/node_modules/$package/dist/index.js"
  # The staged output is shared across lock versions, so its freshness stamp
  # must live next to it, not in the per-version tool_dir: a per-version stamp
  # stays self-consistent after a lock revert and would skip re-staging.
  stamp="$staged_bin.stamp"
  if [[ -x "$staged_bin" && -f "$stamp" && -f "$entrypoint" ]]; then
    # shellcheck disable=SC1090
    source "$stamp"
    # STAMP_INSTALL_MODE distinguishes bridge-only trees from caches staged by
    # the retired full-CLI bundling (whose stamps lack the marker): those trees
    # still vendor the native CLIs and must be reinstalled, not reused.
    if [[ "${STAMP_INSTALL_MODE:-}" == "bridge-only" && "${STAMP_PACKAGE:-}" == "$package" && "${STAMP_VERSION:-}" == "$version" && "${STAMP_INTEGRITY:-}" == "$integrity" && "${STAMP_DEPENDENCY_PACKAGE:-}" == "$dependency_package" && "${STAMP_DEPENDENCY_VERSION:-}" == "$dependency_version" && "${STAMP_DEPENDENCY_INTEGRITY:-}" == "$dependency_integrity" ]]; then
      # The npm tree is fresh, but the compiled launcher tracks the crate,
      # not the lock, so the stamp cannot see it change — refresh it every
      # run (the copy no-ops when already identical).
      if acp_target_is_windows "$target"; then
        write_windows_node_launcher "$staged_bin" "$launcher_exe" "$entrypoint_from_bin_dir" "$node_engine"
      fi
      continue
    fi
  fi

  echo "Installing ACP tool $id $version from npm for $target..." >&2
  rm -rf "$install_dir"
  mkdir -p "$install_dir" "$bin_dir"
  # --omit=optional is what makes the install bridge-only: the native
  # claude/codex CLIs ship as optional platform packages of the pinned
  # dependency, and skipping them keeps the tree pure JS.
  npm install \
    --prefix "$install_dir" \
    --omit=dev \
    --omit=optional \
    --ignore-scripts \
    --no-audit \
    --no-fund \
    "$package@$version" >&2

  validate_npm_install "$install_dir" "$package" "$version" "$integrity" "$dependency_package" "$dependency_version" "$dependency_integrity" "$claude_code_version"
  if acp_target_is_windows "$target"; then
    write_windows_node_launcher "$staged_bin" "$launcher_exe" "$entrypoint_from_bin_dir" "$node_engine"
  else
    write_node_wrapper "$staged_bin" "$entrypoint" "$node_engine"
  fi
  {
    printf 'STAMP_INSTALL_MODE=bridge-only\n'
    printf 'STAMP_TARGET=%q\n' "$target"
    printf 'STAMP_PACKAGE=%q\n' "$package"
    printf 'STAMP_VERSION=%q\n' "$version"
    printf 'STAMP_INTEGRITY=%q\n' "$integrity"
    printf 'STAMP_TARBALL=%q\n' "$tarball"
    printf 'STAMP_NODE_ENGINE=%q\n' "$node_engine"
    printf 'STAMP_DEPENDENCY_PACKAGE=%q\n' "$dependency_package"
    printf 'STAMP_DEPENDENCY_VERSION=%q\n' "$dependency_version"
    printf 'STAMP_DEPENDENCY_INTEGRITY=%q\n' "$dependency_integrity"
    printf 'STAMP_DEPENDENCY_TARBALL=%q\n' "$dependency_tarball"
    printf 'STAMP_CLAUDE_CODE_VERSION=%q\n' "$claude_code_version"
    printf 'STAMP_BINARY=%q\n' "$binary"
  } > "$stamp"
done

# bin_dir is prepended to the agent spawn PATH and the desktop's command
# resolution sweep, so binaries (and stamps) for tools no longer in the lock
# must be pruned, not just left behind.
locked_binaries="$(node -e '
const entries = JSON.parse(process.argv[1]);
for (const entry of entries) console.log(entry.binary);
' "$lock_entries" | sort -u)"
find "$bin_dir" -type f -print0 | while IFS= read -r -d '' staged_file; do
  name="$(basename "$staged_file")"
  # Reduce every staged artifact shape to the lock's bare binary name:
  # <binary>[.exe][.stamp] and the Windows launcher's <binary>.shim.json.
  base="${name%.stamp}"
  base="${base%.shim.json}"
  base="${base%.exe}"
  if ! printf '%s\n' "$locked_binaries" | grep -Fxq -- "$base"; then
    rm -f -- "$staged_file"
  fi
done

if [[ "$print_bin_dir" == "1" ]]; then
  printf '%s\n' "$bin_dir"
fi
