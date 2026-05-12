/**
 * HTTP bridge client for querying Nostr events via POST /query.
 *
 * Uses NIP-98 authentication with the same ephemeral keypair as the
 * WebSocket client (nostr-client.ts).
 */

import {
  generateSecretKey,
  getPublicKey,
  finalizeEvent,
} from "nostr-tools/pure";
import { relayHttpBaseUrl } from "./relay-url";
import type { NostrEvent, NostrFilter } from "./nostr-client";

/** Lazily-generated ephemeral keypair for NIP-98 auth. */
let _httpSecretKey: Uint8Array | null = null;
function getHttpKey(): Uint8Array {
  if (!_httpSecretKey) {
    _httpSecretKey = generateSecretKey();
  }
  return _httpSecretKey;
}

/**
 * SHA-256 hex digest of a string, using the Web Crypto API.
 */
async function sha256Hex(data: string): Promise<string> {
  const buf = new TextEncoder().encode(data);
  const hash = await crypto.subtle.digest("SHA-256", buf);
  return Array.from(new Uint8Array(hash))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

/**
 * Build a NIP-98 Authorization header value.
 *
 * Signs a kind:27235 event with the ephemeral key, base64-encodes it,
 * and returns the full "Nostr <base64>" string.
 *
 * Includes a `["payload", sha256hex(body)]` tag per the NIP-98 spec so
 * that parallel requests with different bodies produce unique auth events.
 */
async function buildNip98Header(
  url: string,
  method: string,
  body: string,
): Promise<string> {
  const sk = getHttpKey();
  const payloadHash = await sha256Hex(body);
  const event = {
    kind: 27235,
    created_at: Math.floor(Date.now() / 1000),
    tags: [
      ["u", url],
      ["method", method],
      ["payload", payloadHash],
    ],
    content: "",
    pubkey: getPublicKey(sk),
  };
  const signed = finalizeEvent(event, sk);
  const json = JSON.stringify(signed);
  const b64 = btoa(json);
  return `Nostr ${b64}`;
}

/**
 * Query events via the HTTP bridge (POST /query).
 *
 * This is used for synthesized ephemeral events (git browse kinds 20100-20103)
 * that are generated on-the-fly by the relay and never stored in the DB.
 */
export async function queryEventsHttp(
  filter: NostrFilter,
): Promise<NostrEvent[]> {
  const url = `${relayHttpBaseUrl()}/query`;
  const body = JSON.stringify([filter]);
  const auth = await buildNip98Header(url, "POST", body);

  const res = await fetch(url, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: auth,
    },
    body,
  });

  if (!res.ok) {
    const errBody = await res.text().catch(() => "");
    throw new Error(
      `HTTP query failed: ${res.status} ${res.statusText}${errBody ? ` — ${errBody}` : ""}`,
    );
  }

  const data: unknown = await res.json();
  if (!Array.isArray(data)) {
    return [];
  }
  return data as NostrEvent[];
}
