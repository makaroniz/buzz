#!/usr/bin/env node
import { execFile } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { promisify } from "node:util";

const appRoot = path.resolve(import.meta.dirname, "..");
const defaultLockFile = path.join(appRoot, "acp-tools.lock.json");

const SUPPORTED_TARGETS = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "aarch64-unknown-linux-gnu",
  "x86_64-unknown-linux-gnu",
  "x86_64-pc-windows-msvc",
];

// The Codex ACP executable stays `codex-acp`, but bundled installs must come
// from the maintained Agent Client Protocol package rather than the stale
// Zed package.
const CODEX_ACP_PACKAGE = "@agentclientprotocol/codex-acp";

// Bridge-only bundling: only the bridge JS trees are pinned and installed
// (`npm install --omit=optional` skips the SDK/codex platform packages that
// vendor the native claude/codex CLIs). The desktop app points each bridge at
// the user's own CLI via CLAUDE_CODE_EXECUTABLE / CODEX_PATH at spawn time.
// The trees are therefore platform-independent, but the lock stays per-target
// so a single target's pins can still be bumped or dropped independently.
const TOOL_SPECS = [
  {
    id: "claude-acp",
    binary: "claude-agent-acp",
    package: "@agentclientprotocol/claude-agent-acp",
    dependencyPackage: "@anthropic-ai/claude-agent-sdk",
    includeClaudeCodeVersion: true,
  },
  {
    id: "codex-acp",
    binary: "codex-acp",
    package: CODEX_ACP_PACKAGE,
    dependencyPackage: "@openai/codex",
  },
];

// Tarball URLs in the lock are informational — ensure-acp-tools.sh installs
// via the ambient npm registry and validates the registry-agnostic sha512
// integrity. Normalize them to the public registry so regenerating against an
// internal mirror (e.g. Artifactory) neither leaks internal hosts into the
// committed lock nor churns every URL in the diff.
const PUBLIC_NPM_REGISTRY = "https://registry.npmjs.org";

const npmViewCache = new Map();
const execFileAsync = promisify(execFile);

function usage() {
  console.log(`Usage: desktop/scripts/update-acp-tools-lock.mjs [--target <triple>]... [--lock-file <path>]

Queries npm for the latest release of each supported ACP bridge tool and
writes acp-tools.lock.json. Only the bridge JS packages are pinned — the
SDK/codex platform packages that vendor native CLIs are optional dependencies
the install omits; the app runs the user's own claude/codex CLI. Fails loudly
when a package or its pinned dependency cannot be resolved — never silently
pins an older version.

A --target run regenerates only the selected targets; the existing lock's
entries for every other target are preserved verbatim, so a partial bump
never drops another target's pins.

Tarball URLs are normalized to ${PUBLIC_NPM_REGISTRY} regardless of the
registry that resolved the metadata, so regeneration is deterministic across
environments and internal mirror hosts never land in the committed lock.

Supported targets:
  ${SUPPORTED_TARGETS.join("\n  ")}

Environment:
  npm registry config     used to resolve packages
  ACP_TOOLS_LOCK_FILE     lockfile path override
`);
}

function parseArgs(argv) {
  const targets = [];
  let lockFile = process.env.ACP_TOOLS_LOCK_FILE ?? defaultLockFile;
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "-h" || arg === "--help") {
      usage();
      process.exit(0);
    }
    if (arg === "--target") {
      const value = argv[++i];
      if (!value) throw new Error("--target requires a value");
      targets.push(value);
      continue;
    }
    if (arg === "--lock-file") {
      const value = argv[++i];
      if (!value) throw new Error("--lock-file requires a value");
      lockFile = path.resolve(value);
      continue;
    }
    throw new Error(`Unknown argument: ${arg}`);
  }

  const selectedTargets = targets.length ? targets : SUPPORTED_TARGETS;
  for (const target of selectedTargets) {
    if (!SUPPORTED_TARGETS.includes(target)) {
      throw new Error(`Unsupported target '${target}'`);
    }
  }
  return { targets: selectedTargets, lockFile };
}

async function npmView(spec, fields) {
  const cacheKey = `${spec}\0${fields.join("\0")}`;
  if (!npmViewCache.has(cacheKey)) {
    npmViewCache.set(
      cacheKey,
      execFileAsync("npm", ["view", spec, ...fields, "--json"], {
        maxBuffer: 10 * 1024 * 1024,
      }).then(({ stdout }) => {
        try {
          return JSON.parse(stdout);
        } catch (error) {
          throw new Error(
            `npm view ${spec} returned invalid JSON: ${
              error instanceof Error ? error.message : String(error)
            }`,
          );
        }
      }),
    );
  }
  return npmViewCache.get(cacheKey);
}

function requireString(value, label) {
  if (typeof value !== "string" || value.trim() === "") {
    throw new Error(`Missing ${label}`);
  }
  return value;
}

// Rebuild the tarball URL on PUBLIC_NPM_REGISTRY, keeping the registry's
// `<package>/-/<basename>.tgz` path rather than deriving the basename from
// name@version — registries own that naming, not us.
function normalizeTarballUrl(tarball, packageName, label) {
  const separator = "/-/";
  const separatorIndex = tarball.lastIndexOf(separator);
  if (separatorIndex === -1) {
    throw new Error(`Cannot normalize ${label} tarball URL: ${tarball}`);
  }
  const basename = tarball.slice(separatorIndex + separator.length);
  return `${PUBLIC_NPM_REGISTRY}/${packageName}/-/${basename}`;
}

function packageDist(metadata, label) {
  const dist = metadata?.dist;
  return {
    tarball: normalizeTarballUrl(
      requireString(dist?.tarball, `${label} dist.tarball`),
      requireString(metadata?.name, `${label} name`),
      label,
    ),
    integrity: requireString(dist?.integrity, `${label} dist.integrity`),
  };
}

function compareSemver(left, right) {
  const leftCore = left.split("-", 1)[0].split(".").map(Number);
  const rightCore = right.split("-", 1)[0].split(".").map(Number);
  for (let i = 0; i < 3; i += 1) {
    if ((leftCore[i] ?? 0) !== (rightCore[i] ?? 0)) {
      return (leftCore[i] ?? 0) - (rightCore[i] ?? 0);
    }
  }
  // A release outranks any prerelease of the same core version.
  return (left.includes("-") ? 0 : 1) - (right.includes("-") ? 0 : 1);
}

// npm view returns a single object when a spec matches one version, but an
// array of per-version objects when a range matches several. Pick the highest
// matching version so a ranged dependency still pins the newest release.
function pickLatestMatch(metadata, label) {
  if (!Array.isArray(metadata)) return metadata;
  if (metadata.length === 0) {
    throw new Error(`No versions match ${label}`);
  }
  return metadata.reduce((best, candidate) =>
    compareSemver(
      requireString(candidate?.version, `${label} version`),
      requireString(best?.version, `${label} version`),
    ) > 0
      ? candidate
      : best,
  );
}

async function lockToolForTarget(tool, target) {
  const packageName = tool.package;
  const packageMetadata = await npmView(`${packageName}@latest`, [
    "name",
    "version",
    "dist",
    "dependencies",
    "engines",
    "bin",
  ]);

  if (packageMetadata.name !== packageName) {
    throw new Error(
      `npm package ${packageName} resolved to ${packageMetadata.name}`,
    );
  }

  const version = requireString(
    packageMetadata.version,
    `${packageName} version`,
  );
  const packageInfo = packageDist(packageMetadata, `${packageName}@${version}`);
  const entry = {
    id: tool.id,
    binary: tool.binary,
    source: "npm",
    package: packageName,
    version,
    integrity: packageInfo.integrity,
    tarball: packageInfo.tarball,
    target,
    nodeEngine: packageMetadata.engines?.node ?? ">=22",
  };

  const dependencyRange = requireString(
    packageMetadata.dependencies?.[tool.dependencyPackage],
    `${packageName} dependency ${tool.dependencyPackage}`,
  );
  const dependencyMetadata = pickLatestMatch(
    await npmView(`${tool.dependencyPackage}@${dependencyRange}`, [
      "name",
      "version",
      "dist",
      "claudeCodeVersion",
    ]),
    `${tool.dependencyPackage}@${dependencyRange}`,
  );
  const dependencyVersion = requireString(
    dependencyMetadata.version,
    `${tool.dependencyPackage}@${dependencyRange} version`,
  );
  const dependencyInfo = packageDist(
    dependencyMetadata,
    `${tool.dependencyPackage}@${dependencyVersion}`,
  );
  entry.dependencyPackage = tool.dependencyPackage;
  entry.dependencyVersion = dependencyVersion;
  entry.dependencyIntegrity = dependencyInfo.integrity;
  entry.dependencyTarball = dependencyInfo.tarball;
  if (tool.includeClaudeCodeVersion) {
    entry.claudeCodeVersion = dependencyMetadata.claudeCodeVersion ?? null;
  }

  return entry;
}

// Entries preserved from the existing lock when --target selects a subset.
// A missing lock preserves nothing (there is nothing to drop); an unreadable
// or malformed one aborts the run rather than clobbering pins we cannot see.
async function preservedLockEntries(lockFile, selectedTargets) {
  let raw;
  try {
    raw = await readFile(lockFile, "utf8");
  } catch (error) {
    if (error?.code === "ENOENT") return [];
    throw new Error(
      `Cannot read existing lock ${lockFile} to preserve unselected targets: ${
        error instanceof Error ? error.message : String(error)
      }`,
    );
  }
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch (error) {
    throw new Error(
      `Existing lock ${lockFile} is not valid JSON — refusing to overwrite it with a partial --target run: ${
        error instanceof Error ? error.message : String(error)
      }`,
    );
  }
  if (!Array.isArray(parsed?.tools)) {
    throw new Error(
      `Existing lock ${lockFile} has no tools array — refusing to overwrite it with a partial --target run`,
    );
  }
  return parsed.tools.filter((entry) => !selectedTargets.has(entry?.target));
}

async function main() {
  const { targets, lockFile } = parseArgs(process.argv.slice(2));
  const selectedTargets = new Set(targets);
  const fullRun = SUPPORTED_TARGETS.every((target) =>
    selectedTargets.has(target),
  );
  const preserved = fullRun
    ? []
    : await preservedLockEntries(lockFile, selectedTargets);
  const tools = [];
  for (const tool of TOOL_SPECS) {
    for (const target of targets) {
      tools.push(await lockToolForTarget(tool, target));
    }
  }
  if (preserved.length) {
    console.log(
      `Preserved ${preserved.length} existing lock entr${
        preserved.length === 1 ? "y" : "ies"
      } for unselected targets`,
    );
  }
  tools.push(...preserved);
  tools.sort((left, right) =>
    `${left.id}:${left.target}`.localeCompare(`${right.id}:${right.target}`),
  );
  await mkdir(path.dirname(lockFile), { recursive: true });
  await writeFile(lockFile, `${JSON.stringify({ tools }, null, 2)}\n`);
  console.log(`Updated ${path.relative(process.cwd(), lockFile)}`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
