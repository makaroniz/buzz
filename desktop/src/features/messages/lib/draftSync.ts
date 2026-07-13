import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import { relayClient } from "@/shared/api/relayClient";
import {
  deriveDraftAddress,
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  relaySupportsNip,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_DRAFT, KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";
import type { DraftState } from "./useDrafts";
import {
  getActiveDraftEntries,
  mergeRemoteDraftEntry,
  removeRemoteDraftEntry,
} from "./useDrafts";
import { parseDraftPayload, serializeDraftPayload } from "./draftPayload";

const DEBOUNCE_MS = 2_000;
const SIDECAR_PREFIX = "buzz-draft-sync.v1";

type RemoteHead = Pick<RelayEvent, "id" | "created_at" | "content">;
type AddressState = {
  draftKey?: string;
  remoteHead?: RemoteHead;
  base?: RemoteHead;
  pendingPublish?: PendingPublish;
  pendingDeletion?: PendingDeletion;
};
type PendingPublish = {
  draftKey: string;
  draft: DraftState;
  channelId: string;
  address?: string;
};
type PendingDeletion = {
  draftKey: string;
  channelId: string;
  address: string | null;
  base?: RemoteHead;
};
type Sidecar = Record<string, PendingDeletion>;

export type DraftSyncDependencies = {
  decrypt?: typeof nip44DecryptFromSelf;
  encrypt?: typeof nip44EncryptToSelf;
  sign?: typeof signRelayEvent;
  deriveAddress?: typeof deriveDraftAddress;
  fetchEvents?: typeof relayClient.fetchEvents;
  publishEvent?: typeof relayClient.publishEvent;
};

function compareHeads(left: RemoteHead, right: RemoteHead): number {
  if (left.created_at !== right.created_at)
    return left.created_at - right.created_at;
  return right.id.localeCompare(left.id);
}

function tagValue(event: RelayEvent, name: string): string | null {
  const tag = event.tags.find((candidate) => candidate[0] === name);
  return tag?.[1] ?? null;
}

export class DraftSyncManager {
  private readonly relayScope: string;
  private readonly state = new Map<string, AddressState>();
  private readonly deps: Required<DraftSyncDependencies>;
  private timer: number | null = null;
  private destroyed = false;
  private unsubscribeReconnect: (() => void) | null = null;
  private liveSubscriptions = new Map<string, () => Promise<void>>();

  private readonly pubkey: string;

  constructor(
    pubkey: string,
    relayUrl: string,
    dependencies: DraftSyncDependencies = {},
  ) {
    this.pubkey = pubkey;
    this.relayScope = normalizeRelayUrl(relayUrl);
    this.deps = {
      decrypt: dependencies.decrypt ?? nip44DecryptFromSelf,
      encrypt: dependencies.encrypt ?? nip44EncryptToSelf,
      sign: dependencies.sign ?? signRelayEvent,
      deriveAddress: dependencies.deriveAddress ?? deriveDraftAddress,
      fetchEvents:
        dependencies.fetchEvents ?? relayClient.fetchEvents.bind(relayClient),
      publishEvent:
        dependencies.publishEvent ?? relayClient.publishEvent.bind(relayClient),
    };
  }

  start(): void {
    this.unsubscribeReconnect ??= relayClient.subscribeToReconnects(() => {
      void this.fetchAllOwnDrafts();
      for (const channelId of this.liveSubscriptions.keys()) {
        void this.fetchOwnDraftsForChannel(channelId);
      }
      void this.replayPendingDeletions();
    });
    void this.fetchAllOwnDrafts();
    void this.replayPendingDeletions();
  }

  queuePublish(draftKey: string, draft: DraftState): void {
    if (this.destroyed || draft.pendingImeta.some((media) => !media.uploaded))
      return;
    const entry = this.state.get(draftKey) ?? {};
    entry.draftKey = draftKey;
    entry.pendingPublish = { draftKey, draft, channelId: draft.channelId };
    this.state.set(draftKey, entry);
    // Resolve the opaque address while the draft is alive so normal delete
    // paths can durably record it before removing visible local state.
    void this.deps
      .deriveAddress(draftKey, this.relayScope)
      .then((address) => {
        const current = this.state.get(draftKey)?.pendingPublish;
        if (current) current.address = address;
      })
      .catch(() => {});
    if (this.timer !== null) window.clearTimeout(this.timer);
    this.timer = window.setTimeout(() => {
      this.timer = null;
      void this.flushPublishes();
    }, DEBOUNCE_MS);
  }

  async queueDeletion(draftKey: string, channelId: string): Promise<void> {
    if (this.destroyed) return;
    if (this.timer !== null) {
      window.clearTimeout(this.timer);
      this.timer = null;
    }
    const entry = this.state.get(draftKey) ?? {};
    entry.draftKey = draftKey;
    const cachedAddress = entry.pendingPublish?.address ?? null;
    const pendingDeletion = {
      draftKey,
      channelId,
      address: cachedAddress,
      base: entry.remoteHead,
    };
    entry.pendingDeletion = pendingDeletion;
    entry.pendingPublish = undefined;
    this.state.set(draftKey, entry);
    // Persist the deletion intent synchronously before visible local state is
    // cleared; derive the opaque address afterward if it was not prewarmed.
    this.writeSidecar();
    if (!pendingDeletion.address) {
      try {
        pendingDeletion.address = await this.deps.deriveAddress(
          draftKey,
          this.relayScope,
        );
      } catch {
        return;
      }
      this.writeSidecar();
    }
    await this.publishTombstone(pendingDeletion);
  }

  async fetchAllOwnDrafts(): Promise<void> {
    await this.fetchAndMerge({
      kinds: [KIND_DRAFT],
      authors: [this.pubkey],
      limit: 500,
    });
  }

  async fetchOwnDraftsForChannel(channelId: string): Promise<void> {
    await this.fetchAndMerge({
      kinds: [KIND_DRAFT],
      authors: [this.pubkey],
      "#h": [channelId],
      limit: 500,
    });
  }

  async subscribeToChannel(channelId: string): Promise<void> {
    if (this.destroyed || this.liveSubscriptions.has(channelId)) return;
    await this.fetchOwnDraftsForChannel(channelId);
    const unsubscribe = await relayClient.subscribeLive(
      {
        kinds: [KIND_DRAFT],
        authors: [this.pubkey],
        "#h": [channelId],
        limit: 0,
      },
      (event) => void this.mergeEvent(event),
    );
    this.liveSubscriptions.set(channelId, unsubscribe);
  }

  async destroy(): Promise<void> {
    this.destroyed = true;
    if (this.timer !== null) {
      window.clearTimeout(this.timer);
      this.timer = null;
      await this.flushPublishes();
    }
    this.unsubscribeReconnect?.();
    this.unsubscribeReconnect = null;
    await Promise.all(
      [...this.liveSubscriptions.values()].map((unsubscribe) => unsubscribe()),
    );
    this.liveSubscriptions.clear();
  }

  private async flushPublishes(): Promise<void> {
    for (const state of this.state.values()) {
      const pending = state.pendingPublish;
      if (!pending || state.pendingDeletion) continue;
      await this.publishDraft(pending, state);
    }
  }

  private async publishDraft(
    pending: PendingPublish,
    state: AddressState,
  ): Promise<void> {
    try {
      const address =
        pending.address ??
        (await this.deps.deriveAddress(pending.draftKey, this.relayScope));
      await this.fetchAndMerge({
        kinds: [KIND_DRAFT],
        authors: [this.pubkey],
        "#d": [address],
        limit: 1,
      });
      if (state.remoteHead?.content === "") return;
      const content = await this.deps.encrypt(
        serializeDraftPayload(pending.draftKey, pending.draft, this.pubkey),
      );
      const event = await this.deps.sign({
        kind: KIND_DRAFT,
        content,
        createdAt: Math.max(
          Math.floor(Date.now() / 1_000),
          (state.remoteHead?.created_at ?? 0) + 1,
        ),
        tags: [
          ["d", address],
          ["h", pending.channelId],
          ["k", String(KIND_STREAM_MESSAGE)],
        ],
      });
      await this.deps.publishEvent(
        event,
        "Timed out publishing draft.",
        "Failed to publish draft.",
      );
      state.base = event;
      state.remoteHead = event;
      state.pendingPublish = undefined;
    } catch (error) {
      console.warn("[draftSync] draft publish failed:", error);
    }
  }

  private async publishTombstone(pending: PendingDeletion): Promise<void> {
    if (!pending.address) return;
    try {
      const event = await this.deps.sign({
        kind: KIND_DRAFT,
        content: "",
        tags: [
          ["d", pending.address],
          ["h", pending.channelId],
          ["k", String(KIND_STREAM_MESSAGE)],
        ],
      });
      await this.deps.publishEvent(
        event,
        "Timed out deleting draft.",
        "Failed to delete draft.",
      );
      const state = this.state.get(pending.draftKey);
      if (state?.pendingDeletion?.address === pending.address) {
        state.pendingDeletion = undefined;
        state.remoteHead = event;
        this.writeSidecar();
      }
    } catch (error) {
      console.warn("[draftSync] draft tombstone failed:", error);
    }
  }

  private async replayPendingDeletions(): Promise<void> {
    for (const pending of Object.values(this.readSidecar())) {
      const state = this.state.get(pending.draftKey) ?? {};
      state.draftKey = pending.draftKey;
      state.pendingDeletion = pending;
      this.state.set(pending.draftKey, state);
      if (!pending.address) {
        try {
          pending.address = await this.deps.deriveAddress(
            pending.draftKey,
            this.relayScope,
          );
        } catch {
          continue;
        }
        this.writeSidecar();
      }
      await this.publishTombstone(pending);
    }
  }

  private async findLocalDraftKeyForAddress(
    address: string,
  ): Promise<string | null> {
    for (const { key } of getActiveDraftEntries()) {
      try {
        if ((await this.deps.deriveAddress(key, this.relayScope)) === address)
          return key;
      } catch {
        // A failed derivation cannot identify a local compose context.
      }
    }
    return null;
  }

  private async fetchAndMerge(filter: {
    kinds: number[];
    authors: string[];
    limit: number;
    "#h"?: string[];
    "#d"?: string[];
  }): Promise<void> {
    try {
      const events = await this.deps.fetchEvents(filter);
      await Promise.all(events.map((event) => this.mergeEvent(event)));
    } catch {
      // Remote failures preserve the local write-through cache.
    }
  }

  private async mergeEvent(event: RelayEvent): Promise<void> {
    if (
      event.pubkey.toLowerCase() !== this.pubkey.toLowerCase() ||
      event.kind !== KIND_DRAFT
    )
      return;
    const address = tagValue(event, "d");
    const channelId = tagValue(event, "h");
    const kind = Number(tagValue(event, "k"));
    if (!address || !channelId || kind !== KIND_STREAM_MESSAGE) return;
    const state = this.state.get(address) ?? {};
    if (state.remoteHead && compareHeads(event, state.remoteHead) <= 0) return;
    if (
      state.pendingDeletion ||
      Object.values(this.readSidecar()).some(
        (entry) => entry.address === address,
      )
    )
      return;
    if (event.content === "") {
      state.remoteHead = event;
      this.state.set(address, state);
      const draftKey =
        state.draftKey ?? (await this.findLocalDraftKeyForAddress(address));
      if (draftKey) removeRemoteDraftEntry(draftKey);
      return;
    }
    try {
      const plaintext = await this.deps.decrypt(event.content);
      const decoded = parseDraftPayload(
        plaintext,
        this.pubkey,
        kind,
        new Date(event.created_at * 1_000).toISOString(),
      );
      if (!decoded || decoded.draft.channelId !== channelId) return;
      if (
        (await this.deps.deriveAddress(decoded.draftKey, this.relayScope)) !==
        address
      )
        return;
      const keyedState = this.state.get(decoded.draftKey);
      const targetState = keyedState ?? state;
      if (
        targetState.remoteHead &&
        compareHeads(event, targetState.remoteHead) < 0
      )
        return;
      targetState.draftKey = decoded.draftKey;
      targetState.remoteHead = event;
      this.state.set(address, targetState);
      this.state.set(decoded.draftKey, targetState);
      if (
        targetState.pendingDeletion ||
        Object.values(this.readSidecar()).some(
          (entry) => entry.draftKey === decoded.draftKey,
        )
      )
        return;
      mergeRemoteDraftEntry(decoded.draftKey, decoded.draft);
    } catch {
      // Untrusted ciphertext must not affect local drafts.
    }
  }

  private sidecarKey(): string {
    return `${SIDECAR_PREFIX}:${this.relayScope}:${this.pubkey}`;
  }
  private readSidecar(): Sidecar {
    try {
      return JSON.parse(
        localStorage.getItem(this.sidecarKey()) ?? "{}",
      ) as Sidecar;
    } catch {
      return {};
    }
  }
  private writeSidecar(): void {
    const sidecar: Sidecar = {};
    for (const entry of this.state.values()) {
      if (entry.pendingDeletion) {
        // The compose key is durable until address derivation returns, then the
        // opaque address becomes the sidecar key. This keeps a just-cleared
        // offline draft from being lost during that asynchronous boundary.
        sidecar[
          entry.pendingDeletion.address ?? entry.pendingDeletion.draftKey
        ] = entry.pendingDeletion;
      }
    }
    localStorage.setItem(this.sidecarKey(), JSON.stringify(sidecar));
  }
}

let activeManager: DraftSyncManager | null = null;
let configurationGeneration = 0;
const activeChannels = new Set<string>();

/** Configure the workspace-scoped singleton without delaying workspace startup. */
export function configureDraftSync(pubkey: string, relayUrl: string): void {
  const generation = ++configurationGeneration;
  void (async () => {
    try {
      if (
        !(await relaySupportsNip(37)) ||
        generation !== configurationGeneration
      )
        return;
      activeManager?.destroy().catch(() => {});
      activeManager = new DraftSyncManager(pubkey, relayUrl);
      activeManager.start();
      for (const channelId of activeChannels)
        void activeManager.subscribeToChannel(channelId);
    } catch {
      // Capability probe failures deliberately leave draft behavior local-only.
    }
  })();
}

export function resetDraftSync(): void {
  configurationGeneration += 1;
  activeManager?.destroy().catch(() => {});
  activeManager = null;
  activeChannels.clear();
}

export function syncPersistedDraft(draftKey: string, draft: DraftState): void {
  activeManager?.queuePublish(draftKey, draft);
}

export function syncDeletedDraft(draftKey: string, channelId: string): void {
  void activeManager?.queueDeletion(draftKey, channelId);
}

export function syncDraftChannel(channelId: string): void {
  activeChannels.add(channelId);
  void activeManager?.subscribeToChannel(channelId);
}

export function backfillSyncedDrafts(): void {
  void activeManager?.fetchAllOwnDrafts();
}
