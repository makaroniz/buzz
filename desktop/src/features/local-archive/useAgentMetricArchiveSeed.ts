/**
 * First-run seeding for agent-turn-metric archive.
 *
 * When an internal build has `BUZZ_BUILD_AGENT_METRIC_ARCHIVE_DEFAULT` set and
 * the current identity has not yet made an explicit choice, this hook
 * auto-creates an `owner_p` save subscription including kind 44200 agent turn
 * metrics, scoped to the current identity's pubkey.
 *
 * Merges with any existing `owner_p` subscription (e.g. an observer
 * subscription already seeded) rather than overwriting, so both kinds coexist
 * in one row.
 *
 * OSS builds return `false` from `agent_metric_archive_default_enabled` →
 * no-op. After any explicit user action (seeding or opt-out), the localStorage
 * flag prevents re-seeding on subsequent starts.
 */

import * as React from "react";

import { KIND_AGENT_TURN_METRIC } from "@/shared/constants/kinds";
import {
  createSaveSubscription,
  listSaveSubscriptions,
  agentMetricArchiveDefaultEnabled,
} from "@/shared/api/tauriArchive";
import {
  hasExplicitAgentMetricArchiveChoice,
  setExplicitAgentMetricArchiveChoice,
} from "./agentMetricArchivePreference";

/**
 * Deps interface for testing.  Production callers pass nothing.
 */
export interface AgentMetricArchiveSeedDeps {
  agentMetricArchiveDefaultEnabled: () => Promise<boolean>;
  listSaveSubscriptions: () => Promise<
    Array<{ scopeType: string; kinds: number[] }>
  >;
  createSaveSubscription: (
    scopeType: "owner_p",
    scopeValue: string,
    kinds: number[],
  ) => Promise<void>;
  hasExplicitChoice: (pubkey: string) => boolean;
  setExplicitChoice: (pubkey: string, enabled: boolean) => void;
}

const defaultDeps: AgentMetricArchiveSeedDeps = {
  agentMetricArchiveDefaultEnabled,
  listSaveSubscriptions,
  createSaveSubscription,
  hasExplicitChoice: hasExplicitAgentMetricArchiveChoice,
  setExplicitChoice: setExplicitAgentMetricArchiveChoice,
};

/**
 * Seed the agent-turn-metric archive subscription for `pubkey` once per
 * identity per device on internal builds.
 *
 * @param pubkey - current identity pubkey.  When undefined (identity not yet
 *   loaded), the hook waits until it becomes available.
 * @param deps - optional dep-injection for tests.
 */
export function useAgentMetricArchiveSeed(
  pubkey: string | undefined,
  deps: AgentMetricArchiveSeedDeps = defaultDeps,
): void {
  React.useEffect(() => {
    if (!pubkey) return;

    // Already made an explicit choice for this identity — never re-seed.
    if (deps.hasExplicitChoice(pubkey)) return;

    let cancelled = false;

    async function maybeSeed(): Promise<void> {
      // pubkey is checked above but TypeScript doesn't narrow across the async
      // boundary — re-guard here so the call below is type-safe.
      if (!pubkey) return;

      let defaultOn: boolean;
      try {
        defaultOn = await deps.agentMetricArchiveDefaultEnabled();
      } catch (err) {
        console.warn("[useAgentMetricArchiveSeed] flag check failed:", err);
        return;
      }

      if (cancelled) return;

      if (!defaultOn) {
        // OSS build (flag off): don't persist a choice — leave null so seeding
        // can still fire if this identity later runs an internal build.
        return;
      }

      // Internal build + no prior choice → auto-seed.
      try {
        // Merge with any existing owner_p subscription so a concurrently-seeded
        // observer subscription (24200) is not overwritten.
        let existingKinds: number[] = [];
        try {
          const existing = await deps.listSaveSubscriptions();
          existingKinds =
            existing.find((s) => s.scopeType === "owner_p")?.kinds ?? [];
        } catch {
          // Best-effort — on error, seed with just our kind.
        }
        const mergedKinds = existingKinds.includes(KIND_AGENT_TURN_METRIC)
          ? existingKinds
          : [...existingKinds, KIND_AGENT_TURN_METRIC];
        await deps.createSaveSubscription("owner_p", pubkey, mergedKinds);
      } catch (err) {
        console.warn(
          "[useAgentMetricArchiveSeed] createSaveSubscription failed:",
          err,
        );
        // Do NOT set the localStorage flag — a transient failure (relay
        // unreachable, archive DB not yet initialized) should retry on next
        // startup rather than permanently suppress seeding.
        return;
      }

      if (cancelled) return;

      // Persist the explicit choice so this never re-fires.
      deps.setExplicitChoice(pubkey, true);
    }

    void maybeSeed();

    return () => {
      cancelled = true;
    };
  }, [pubkey, deps]);
}
