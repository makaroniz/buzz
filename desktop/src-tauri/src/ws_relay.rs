//! Serverless-mode relay transport over a plain WebSocket.
//!
//! In serverless mode the desktop app talks to a generic public Nostr relay
//! that has no Sprout HTTP bridge (`/query`, `/events`), no Postgres, and no
//! NIP-98 auth. This module provides the WebSocket equivalents:
//!
//! - [`query_relay_ws`] — one-shot `REQ` / collect until `EOSE` / `CLOSE`.
//! - [`submit_event_ws`] — `EVENT` publish, wait for `OK`.
//!
//! Both perform NIP-42 AUTH only if the relay challenges (most public relays
//! do not). The signing key is the user's identity key from [`AppState`].
//!
//! These mirror the HTTP helpers in `relay.rs` so the rest of the codebase
//! (channels, DMs, agents) is transport-agnostic: it calls `query_relay` /
//! `submit_event`, which dispatch here when `state.is_serverless()`.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::EventBuilder;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::app_state::AppState;
use crate::relay::SubmitEventResponse;

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(10);

/// Execute one or more filters as a single `REQ` and collect matching events
/// until the relay sends `EOSE`. Mirrors `relay::query_relay` but over a plain
/// WebSocket against a generic relay.
/// Query a set of relays concurrently and merge results, deduplicating events
/// by id. Succeeds if any relay responds; errors only if all fail.
pub async fn query_relay_ws(
    state: &AppState,
    relay_urls: &[String],
    filters: &[serde_json::Value],
) -> Result<Vec<nostr::Event>, String> {
    let futures = relay_urls
        .iter()
        .map(|url| query_relay_ws_one(state, url, filters));
    let results = futures_util::future::join_all(futures).await;

    let mut by_id: std::collections::HashMap<String, nostr::Event> =
        std::collections::HashMap::new();
    let mut last_err = None;
    let mut any_ok = false;
    for r in results {
        match r {
            Ok(events) => {
                any_ok = true;
                for ev in events {
                    by_id.entry(ev.id.to_hex()).or_insert(ev);
                }
            }
            Err(e) => last_err = Some(e),
        }
    }
    if !any_ok {
        return Err(last_err.unwrap_or_else(|| "all relays failed".to_string()));
    }
    Ok(by_id.into_values().collect())
}

async fn query_relay_ws_one(
    state: &AppState,
    relay_url: &str,
    filters: &[serde_json::Value],
) -> Result<Vec<nostr::Event>, String> {
    let keys = {
        let guard = state.keys.lock().map_err(|e| e.to_string())?;
        guard.clone()
    };

    let (ws, _) = connect_async(relay_url)
        .await
        .map_err(|e| format!("relay connection failed: {e}"))?;
    let (mut write, mut read) = ws.split();

    // Build ["REQ", <sub>, <filter>, <filter>, ...]
    let sub_id = format!("q-{}", uuid::Uuid::new_v4());
    let mut req = vec![
        serde_json::Value::String("REQ".into()),
        serde_json::Value::String(sub_id.clone()),
    ];
    req.extend(filters.iter().cloned());
    let req_json = serde_json::Value::Array(req).to_string();

    write
        .send(Message::Text(req_json.into()))
        .await
        .map_err(|e| format!("failed to send REQ: {e}"))?;

    let mut events: Vec<nostr::Event> = Vec::new();

    let collect = tokio::time::timeout(QUERY_TIMEOUT, async {
        loop {
            let msg = match read.next().await {
                Some(Ok(m)) => m,
                Some(Err(e)) => return Err(format!("WS read error: {e}")),
                None => return Err("relay closed during query".to_string()),
            };
            let Message::Text(text) = msg else { continue };
            let Ok(arr) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let Some(arr) = arr.as_array() else { continue };
            let Some(tag) = arr.first().and_then(|v| v.as_str()) else {
                continue;
            };

            // For EVENT/EOSE/CLOSED, arr[1] is the subscription id.
            let sub_matches = arr.get(1).and_then(|v| v.as_str()) == Some(sub_id.as_str());

            match tag {
                "EVENT" if sub_matches => {
                    // ["EVENT", <sub>, <event>]
                    if let Some(ev) = arr
                        .get(2)
                        .and_then(|v| serde_json::from_value::<nostr::Event>(v.clone()).ok())
                    {
                        events.push(ev);
                    }
                }
                "EOSE" if sub_matches => return Ok(()),
                "CLOSED" if sub_matches => {
                    let reason = arr
                        .get(2)
                        .and_then(|v| v.as_str())
                        .unwrap_or("subscription closed by relay");
                    return Err(format!("relay closed subscription: {reason}"));
                }
                "AUTH" => {
                    // Relay wants NIP-42 auth. Sign and send, then keep reading.
                    if let Some(challenge) = arr.get(1).and_then(|v| v.as_str()) {
                        if let Ok(auth_json) = build_auth_message(&keys, relay_url, challenge) {
                            let _ = write.send(Message::Text(auth_json.into())).await;
                        }
                    }
                }
                _ => {}
            }
        }
    })
    .await;

    // Best-effort CLOSE so we don't leave a dangling sub.
    let close_json = serde_json::json!(["CLOSE", sub_id]).to_string();
    let _ = write.send(Message::Text(close_json.into())).await;
    let _ = write.close().await;

    match collect {
        Ok(Ok(())) => Ok(events),
        Ok(Err(e)) => {
            // A relay that CLOSED for auth reasons but we still got some events:
            // return what we have rather than failing hard.
            if !events.is_empty() {
                Ok(events)
            } else {
                Err(e)
            }
        }
        Err(_) => {
            // Timed out waiting for EOSE — return whatever arrived. Many public
            // relays are slow to EOSE; partial results are better than nothing.
            Ok(events)
        }
    }
}

/// Publish a signed event over a plain WebSocket and wait for the relay's
/// `OK` acknowledgement. Mirrors `relay::submit_event`.
/// Sign once, then publish to all relays. Succeeds if any relay accepts.
pub async fn submit_event_ws(
    builder: EventBuilder,
    state: &AppState,
    relay_urls: &[String],
) -> Result<SubmitEventResponse, String> {
    let keys = {
        let guard = state.keys.lock().map_err(|e| e.to_string())?;
        guard.clone()
    };
    let event = builder
        .sign_with_keys(&keys)
        .map_err(|e| format!("failed to sign event: {e}"))?;

    let futures = relay_urls
        .iter()
        .map(|url| submit_event_ws_one(&event, &keys, url));
    let results = futures_util::future::join_all(futures).await;

    let mut last_err = None;
    for r in results {
        match r {
            Ok(resp) if resp.accepted => return Ok(resp),
            Ok(resp) => last_err = Some(format!("relay rejected event: {}", resp.message)),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| "all relays failed".to_string()))
}

async fn submit_event_ws_one(
    event: &nostr::Event,
    keys: &nostr::Keys,
    relay_url: &str,
) -> Result<SubmitEventResponse, String> {
    let event_id = event.id.to_hex();
    let event_json = serde_json::json!(["EVENT", event]).to_string();

    let (ws, _) = connect_async(relay_url)
        .await
        .map_err(|e| format!("relay connection failed: {e}"))?;
    let (mut write, mut read) = ws.split();

    write
        .send(Message::Text(event_json.clone().into()))
        .await
        .map_err(|e| format!("failed to send EVENT: {e}"))?;

    let result = tokio::time::timeout(PUBLISH_TIMEOUT, async {
        loop {
            let msg = match read.next().await {
                Some(Ok(m)) => m,
                Some(Err(e)) => return Err(format!("WS read error: {e}")),
                None => return Err("relay closed during publish".to_string()),
            };
            let Message::Text(text) = msg else { continue };
            let Ok(arr) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let Some(arr) = arr.as_array() else { continue };
            let Some(tag) = arr.first().and_then(|v| v.as_str()) else {
                continue;
            };

            match tag {
                // ["OK", <event_id>, <accepted: bool>, <message>]
                "OK" if arr.get(1).and_then(|v| v.as_str()) == Some(event_id.as_str()) => {
                    let accepted = arr.get(2).and_then(|v| v.as_bool()).unwrap_or(false);
                    let message = arr
                        .get(3)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    return Ok(SubmitEventResponse {
                        event_id: event_id.clone(),
                        accepted,
                        message,
                    });
                }
                "AUTH" => {
                    if let Some(challenge) = arr.get(1).and_then(|v| v.as_str()) {
                        if let Ok(auth_json) = build_auth_message(keys, relay_url, challenge) {
                            let _ = write.send(Message::Text(auth_json.into())).await;
                            // Re-send the event after authenticating.
                            let _ = write.send(Message::Text(event_json.clone().into())).await;
                        }
                    }
                }
                _ => {}
            }
        }
    })
    .await;

    let _ = write.close().await;

    match result {
        Ok(Ok(resp)) => {
            if !resp.accepted {
                return Err(format!("relay rejected event: {}", resp.message));
            }
            Ok(resp)
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            // Many relays accept silently or are slow to OK. Treat a timeout as
            // best-effort success so writes don't spuriously fail in the UI.
            Ok(SubmitEventResponse {
                event_id,
                accepted: true,
                message: "published (no OK received before timeout)".to_string(),
            })
        }
    }
}

/// Publish an already-signed event over a plain WebSocket and wait for `OK`.
///
/// Unlike [`submit_event_ws`], this takes a pre-signed event and the keys that
/// signed it (used to answer a NIP-42 AUTH challenge). Used for serverless
/// agent-profile sync, where the event is signed by the agent's keys rather
/// than the user's identity key.
pub async fn publish_signed_event_ws(
    event: &nostr::Event,
    keys: &nostr::Keys,
    relay_urls: &[String],
) -> Result<(), String> {
    let futures = relay_urls
        .iter()
        .map(|url| publish_signed_event_ws_one(event, keys, url));
    let results = futures_util::future::join_all(futures).await;
    let mut last_err = None;
    for r in results {
        match r {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| "all relays failed".to_string()))
}

async fn publish_signed_event_ws_one(
    event: &nostr::Event,
    keys: &nostr::Keys,
    relay_url: &str,
) -> Result<(), String> {
    let event_id = event.id.to_hex();
    let event_json = serde_json::json!(["EVENT", event]).to_string();

    let (ws, _) = connect_async(relay_url)
        .await
        .map_err(|e| format!("relay connection failed: {e}"))?;
    let (mut write, mut read) = ws.split();

    write
        .send(Message::Text(event_json.clone().into()))
        .await
        .map_err(|e| format!("failed to send EVENT: {e}"))?;

    let result = tokio::time::timeout(PUBLISH_TIMEOUT, async {
        loop {
            let msg = match read.next().await {
                Some(Ok(m)) => m,
                Some(Err(e)) => return Err(format!("WS read error: {e}")),
                None => return Err("relay closed during publish".to_string()),
            };
            let Message::Text(text) = msg else { continue };
            let Ok(arr) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let Some(arr) = arr.as_array() else { continue };
            let Some(tag) = arr.first().and_then(|v| v.as_str()) else {
                continue;
            };
            match tag {
                "OK" if arr.get(1).and_then(|v| v.as_str()) == Some(event_id.as_str()) => {
                    return Ok(());
                }
                "AUTH" => {
                    if let Some(challenge) = arr.get(1).and_then(|v| v.as_str()) {
                        if let Ok(auth_json) = build_auth_message(keys, relay_url, challenge) {
                            let _ = write.send(Message::Text(auth_json.into())).await;
                            let _ = write.send(Message::Text(event_json.clone().into())).await;
                        }
                    }
                }
                _ => {}
            }
        }
    })
    .await;

    let _ = write.close().await;

    match result {
        Ok(Ok(())) | Err(_) => Ok(()), // best-effort: tolerate slow/silent relays
        Ok(Err(e)) => Err(e),
    }
}

/// Build a NIP-42 `["AUTH", <event>]` message string.
fn build_auth_message(
    keys: &nostr::Keys,
    relay_url: &str,
    challenge: &str,
) -> Result<String, String> {
    let url = nostr::RelayUrl::parse(relay_url).map_err(|e| format!("invalid relay URL: {e}"))?;
    let event = EventBuilder::auth(challenge.to_string(), url)
        .sign_with_keys(keys)
        .map_err(|e| format!("failed to sign auth event: {e}"))?;
    Ok(serde_json::json!(["AUTH", event]).to_string())
}
