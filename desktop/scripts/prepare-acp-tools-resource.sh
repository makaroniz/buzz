#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app_root="$(cd "$script_dir/.." && pwd)"
lock_file="${ACP_TOOLS_LOCK_FILE:-$app_root/acp-tools.lock.json}"

# shellcheck source=lib/acp-node-wrapper.sh
source "$script_dir/lib/acp-node-wrapper.sh"

usage() {
  cat <<'USAGE'
Usage: desktop/scripts/prepare-acp-tools-resource.sh [target-triple]

Stages the locked ACP bridge tools into src-tauri/resources/acp so Tauri can
bundle them as application resources: vendored npm package trees under
resources/acp/node and executable wrappers under resources/acp/bin (bash
shims on Unix targets; the compiled buzz-acp-node-launcher plus
<binary>.shim.json manifests on Windows targets). The optional target triple
defaults to the Rust host target.

Note: resources/acp/bin holds a single target at a time, so staging must stay
tied to the build target.
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

target="${1:-}"
ensure_args=()
if [[ -n "$target" ]]; then
  ensure_args+=(--target "$target")
else
  target="$(rustc -vV | sed -n 's|host: ||p')"
fi
if [[ -z "$target" ]]; then
  echo "Could not determine rust host target." >&2
  exit 1
fi

cache_bin_dir="$("$script_dir/ensure-acp-tools.sh" ${ensure_args[@]+"${ensure_args[@]}"} --print-bin-dir)"
cache_root="$(dirname "$(dirname "$cache_bin_dir")")"

# Windows targets stage the compiled launcher shim instead of a bash
# wrapper. Resolved lazily at first use so a target with no locked tools
# stays a no-op stage; ensure-acp-tools.sh above already built the launcher
# for any target that has them, so resolution hits a warm target dir.
launcher_exe=""
resource_root="$app_root/src-tauri/resources/acp"
resource_bin_dir="$resource_root/bin"
resource_node_dir="$resource_root/node"
mkdir -p "$resource_bin_dir"

# Keep .gitkeep but refresh any staged tools from the lock.
find "$resource_bin_dir" -type f ! -name ".gitkeep" -delete
rm -rf "$resource_node_dir"
mkdir -p "$resource_node_dir"

# Manifest for the app's Node.js runtime doctor check, staged next to the
# bin dir so the app can resolve it as the bin dir's parent. Removed up
# front so locks with no npm-sourced tools ship no manifest and the doctor
# check stays silent.
node_runtime_manifest="$resource_root/node-runtime.json"
rm -f "$node_runtime_manifest"
node_runtime_entries=()

# Stale artifact of the retired full-CLI bundling: the harness CLI manifest
# pointed auth probes at CLIs vendored inside the bundles. Bridge-only
# bundles carry no CLIs, so remove any leftover manifest from an earlier
# stage — it is unread, but it would otherwise ride into the app bundle.
rm -f "$resource_root/harness-clis.json"

# Ad-hoc signing failure is a warning, not a hard stop: an unsignable Mach-O
# fragment that never executes should not sink the stage, and release builds
# re-sign everything with the real identity anyway. But it must be visible —
# a silently unsigned binary surfaces much later as Gatekeeper killing a
# subprocess mid-session, which is undiagnosable from build output.
codesign_if_darwin() {
  local file="$1"
  local output
  if [[ "$(uname -s)" == "Darwin" ]] && command -v codesign >/dev/null 2>&1; then
    if ! output="$(codesign --force --sign - "$file" 2>&1)"; then
      echo "Warning: ad-hoc codesign failed for $file — Gatekeeper may kill it at spawn time:" >&2
      echo "$output" >&2
    fi
  fi
}

while IFS=$'\t' read -r id binary package version node_engine; do
  [[ -n "$id" ]] || continue
  install_dir="$cache_root/$target/$id/$version/npm"
  entrypoint="$install_dir/node_modules/$package/dist/index.js"
  if [[ ! -f "$entrypoint" ]]; then
    echo "Locked npm ACP tool missing from cache: $package@$version" >&2
    exit 1
  fi
  resource_package_dir="$resource_node_dir/$id"
  mkdir -p "$resource_package_dir"
  cp -R "$install_dir/." "$resource_package_dir/"
  resource_entrypoint="$resource_package_dir/node_modules/$package/dist/index.js"
  if [[ ! -f "$resource_entrypoint" ]]; then
    echo "Failed to stage npm ACP tool: $package@$version" >&2
    exit 1
  fi
  if acp_target_is_windows "$target"; then
    if [[ -z "$launcher_exe" ]]; then
      launcher_exe="$(acp_node_launcher_exe "$target")"
    fi
    write_windows_node_launcher "$resource_bin_dir/$(acp_staged_binary_name "$binary" "$target")" "$launcher_exe" "../node/$id/node_modules/$package/dist/index.js" "$node_engine"
  else
    write_node_wrapper "$resource_bin_dir/$binary" "../node/$id/node_modules/$package/dist/index.js" "$node_engine"
  fi
  node_runtime_entries+=("$id"$'\t'"$binary"$'\t'"$node_engine"$'\t'"$(acp_required_node_major "$node_engine")")
  # Ad-hoc sign every Mach-O in the staged package. Bridge-only trees are
  # pure JS, so this scan normally finds nothing — kept because unsigned
  # nested Mach-Os are killed by Gatekeeper, and a future dependency could
  # reintroduce one. Darwin only, so Linux staging skips the file(1) scan.
  if [[ "$(uname -s)" == "Darwin" ]]; then
    while IFS= read -r -d '' candidate; do
      if file -b "$candidate" | grep -q "Mach-O"; then
        codesign_if_darwin "$candidate"
      fi
    done < <(find "$resource_package_dir" -type f -print0)
  fi
done < <(node - "$lock_file" "$target" <<'NODE'
const fs = require("node:fs");
const [lockFile, target] = process.argv.slice(2);
const data = JSON.parse(fs.readFileSync(lockFile, "utf8"));
for (const entry of data.tools ?? []) {
  if (entry.target !== target || typeof entry.binary !== "string") continue;
  if (entry.source !== "npm") {
    throw new Error(`Unsupported ACP tool source: ${entry.source}`);
  }
  console.log([entry.id, entry.binary, entry.package, entry.version, entry.nodeEngine ?? ">=22"].join("\t"));
}
NODE
)

# One manifest entry per npm-sourced bridge, each carrying its own required
# Node major, so bridges with different engine ranges surface distinct
# requirements in the doctor check.
if ((${#node_runtime_entries[@]} > 0)); then
  node -e '
const fs = require("node:fs");
const [manifestFile, ...entries] = process.argv.slice(1);
const tools = entries.map((line) => {
  const [id, binary, nodeEngine, requiredNodeMajor] = line.split("\t");
  return { id, binary, nodeEngine, requiredNodeMajor: Number(requiredNodeMajor) };
});
fs.writeFileSync(manifestFile, `${JSON.stringify({ tools }, null, 2)}\n`);
' "$node_runtime_manifest" ${node_runtime_entries[@]+"${node_runtime_entries[@]}"}
  echo "Wrote ACP Node runtime manifest: $node_runtime_manifest"
fi

echo "Staged ACP tools resource: $resource_bin_dir"
