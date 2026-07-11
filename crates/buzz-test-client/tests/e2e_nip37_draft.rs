//! End-to-end integration tests for NIP-37 draft wraps (kind:31234),
//! channel-bound contract.
//!
//! Every kind:31234 must carry exactly one `h` UUID binding it to a Buzz
//! channel (or DM).  The relay enforces:
//!
//! - Structural: valid `d`/`k`/`h` tags, `p` forbidden
//! - Channel existence: `h` UUID must resolve to a live channel
//! - Membership: author must be a member of that channel
//! - Immutable binding: once written, the `h` tag is frozen per (author, d_tag)
//! - Author-only reads: REQ, WS COUNT, WS subscription, HTTP /query, /count
//! - FTS exclusion: search_tsv = NULL, never surfaces in NIP-50 results
//! - Workflow exclusion: draft events must not appear in workflow triggers
//! - NIP-11 advertisement: relay claims NIP-37
//!
//! # Running
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client --test e2e_nip37_draft -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{EventBuilder, Filter, Keys, Kind, Tag, Timestamp};
use reqwest::Client;
use serde_json::{json, Value};

const KIND_DRAFT: u16 = 31234;
const KIND_CREATE_CHANNEL: u16 = 9007;
const KIND_PUT_USER: u16 = 9000;
const KIND_REMOVE_USER: u16 = 9001;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn relay_http_url() -> String {
    relay_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

fn sub_id(name: &str) -> String {
    format!("e2e-nip37-{name}-{}", uuid::Uuid::new_v4())
}

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
}

/// Minimal syntactically-plausible NIP-44 v2 payload.
/// base64(b"\x02" + b"\x00" * 98) — 132 chars, decoded 99 bytes, first byte 0x02.
fn fake_nip44_v2() -> String {
    let mut s = String::from("Ag");
    s.push_str(&"A".repeat(130));
    s
}

/// Create an open channel as `owner`; returns the channel UUID string.
async fn create_open_channel(owner: &Keys) -> String {
    let client = http_client();
    let ch_id = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_CREATE_CHANNEL), "")
        .tags([
            Tag::parse(["h", &ch_id]).unwrap(),
            Tag::parse(["name", &format!("nip37-test-{ch_id}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &owner.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("create channel");
    let body: Value = resp.json().await.expect("parse channel response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "channel creation not accepted: {body}"
    );
    ch_id
}

/// Create a private channel as `owner`; returns the channel UUID string.
async fn create_private_channel(owner: &Keys) -> String {
    let client = http_client();
    let ch_id = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_CREATE_CHANNEL), "")
        .tags([
            Tag::parse(["h", &ch_id]).unwrap(),
            Tag::parse(["name", &format!("nip37-priv-{ch_id}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "private"]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &owner.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("create private channel");
    let body: Value = resp.json().await.expect("parse channel response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "private channel creation not accepted: {body}"
    );
    ch_id
}

/// Add `member` to a channel via kind:9000 submitted by `owner` over HTTP.
async fn add_member_http(client: &Client, owner: &Keys, channel_id: &str, member: &Keys) {
    let event = EventBuilder::new(Kind::Custom(KIND_PUT_USER), "")
        .tags([
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["p", &member.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &owner.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("add member");
    let body: Value = resp.json().await.expect("parse add-member response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "add member not accepted: {body}"
    );
}

/// Remove `member` from a channel via kind:9001 submitted by `owner` over HTTP.
async fn remove_member_http(client: &Client, owner: &Keys, channel_id: &str, member: &Keys) {
    let event = EventBuilder::new(Kind::Custom(KIND_REMOVE_USER), "")
        .tags([
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["p", &member.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &owner.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("remove member");
    let body: Value = resp.json().await.expect("parse remove-member response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "remove member not accepted: {body}"
    );
}

/// Submit an event via the HTTP bridge and return (accepted, message).
async fn submit_event_http(client: &Client, keys: &Keys, event: &nostr::Event) -> (bool, String) {
    let pubkey_hex = keys.public_key().to_hex();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(event).unwrap())
        .send()
        .await
        .expect("submit event");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("parse response");
    if status == 200 {
        let accepted = body["accepted"].as_bool().unwrap_or(false);
        let message = body["message"].as_str().unwrap_or("").to_string();
        (accepted, message)
    } else {
        let message = body["error"].as_str().unwrap_or("").to_string();
        (false, message)
    }
}

/// Query events via HTTP bridge as `as_pubkey_hex`. Returns events array.
async fn query_events_http(
    client: &Client,
    as_pubkey_hex: &str,
    filters: Vec<Filter>,
) -> Vec<Value> {
    let resp = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", as_pubkey_hex)
        .header("Content-Type", "application/json")
        .json(&filters)
        .send()
        .await
        .expect("query events");
    assert!(
        resp.status().is_success(),
        "query failed: {}",
        resp.status()
    );
    resp.json::<Vec<Value>>()
        .await
        .expect("parse query response")
}

/// Build a valid kind:31234 draft wrap event bound to `channel_id`.
fn build_draft(
    keys: &Keys,
    d_tag: &str,
    k_val: &str,
    channel_id: &str,
    content: &str,
) -> nostr::Event {
    build_draft_at(keys, d_tag, k_val, channel_id, content, Timestamp::now())
}

/// Build a valid kind:31234 draft wrap event bound to `channel_id` at `ts`.
fn build_draft_at(
    keys: &Keys,
    d_tag: &str,
    k_val: &str,
    channel_id: &str,
    content: &str,
    ts: Timestamp,
) -> nostr::Event {
    EventBuilder::new(Kind::Custom(KIND_DRAFT), content)
        .tags([
            Tag::parse(["d", d_tag]).unwrap(),
            Tag::parse(["k", k_val]).unwrap(),
            Tag::parse(["h", channel_id]).unwrap(),
        ])
        .custom_created_at(ts)
        .sign_with_keys(keys)
        .unwrap()
}

/// Build a blank-content tombstone (NIP-37 deletion) bound to `channel_id`.
fn build_tombstone(
    keys: &Keys,
    d_tag: &str,
    k_val: &str,
    channel_id: &str,
    ts: Timestamp,
) -> nostr::Event {
    build_draft_at(keys, d_tag, k_val, channel_id, "", ts)
}

// ─── h-tag validation ─────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_rejected_missing_h_tag() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            // no h tag
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "missing h tag should be rejected");
    assert!(
        msg.contains("h` tag") || msg.contains("channel-bound"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_duplicate_h_tag() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let ch = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch]).unwrap(),
            Tag::parse(["h", &ch]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "duplicate h tag should be rejected");
    assert!(
        msg.contains("h` tag") || msg.contains("channel-bound"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_non_uuid_h_tag() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", "not-a-uuid"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "non-UUID h tag should be rejected");
    assert!(
        msg.contains("UUID") || msg.contains("h` tag"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_nonexistent_channel_h_tag() {
    // h tag is a syntactically valid UUID, but no channel exists for it.
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let nonexistent_ch = uuid::Uuid::new_v4().to_string();
    let event = build_draft(&keys, &d, "9", &nonexistent_ch, &fake_nip44_v2());
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "draft to nonexistent channel should be rejected");
    assert!(
        msg.contains("channel") || msg.contains("not found") || msg.contains("member"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_non_member_author() {
    // Channel exists but author is not a member.
    let client = http_client();
    let owner = Keys::generate();
    let non_member = Keys::generate();

    let ch_id = create_private_channel(&owner).await;

    let d = uuid::Uuid::new_v4().to_string();
    let event = build_draft(&non_member, &d, "9", &ch_id, &fake_nip44_v2());
    let (accepted, msg) = submit_event_http(&client, &non_member, &event).await;
    assert!(
        !accepted,
        "non-member should be unable to post draft: {msg}"
    );
    assert!(
        msg.contains("member") || msg.contains("restricted"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_accepted_by_channel_member() {
    // Channel owner is always a member — their draft must be accepted.
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;

    let d = uuid::Uuid::new_v4().to_string();
    let event = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(accepted, "owner draft must be accepted: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_after_member_removed() {
    // Member writes a draft; gets removed; attempts a replacement — must be rejected.
    let client = http_client();
    let owner = Keys::generate();
    let member = Keys::generate();
    let ch_id = create_private_channel(&owner).await;
    add_member_http(&client, &owner, &ch_id, &member).await;

    let d = uuid::Uuid::new_v4().to_string();
    let now = Timestamp::now().as_secs();
    let v1 = build_draft_at(
        &member,
        &d,
        "9",
        &ch_id,
        &fake_nip44_v2(),
        Timestamp::from(now - 1),
    );
    let (ok1, msg1) = submit_event_http(&client, &member, &v1).await;
    assert!(ok1, "member draft v1 must be accepted: {msg1}");

    remove_member_http(&client, &owner, &ch_id, &member).await;

    let v2 = build_draft(&member, &d, "9", &ch_id, &fake_nip44_v2());
    let (accepted, msg) = submit_event_http(&client, &member, &v2).await;
    assert!(
        !accepted,
        "removed member should not be able to update draft: {msg}"
    );
    assert!(
        msg.contains("member") || msg.contains("restricted"),
        "unexpected message: {msg}"
    );
}

// ─── Immutable channel binding ────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_channel_binding_is_immutable() {
    // Once a draft is bound to channel A, updating it with h=B must be rejected.
    let client = http_client();
    let owner = Keys::generate();
    let ch_a = create_open_channel(&owner).await;
    let ch_b = create_open_channel(&owner).await;

    let d = uuid::Uuid::new_v4().to_string();
    let now = Timestamp::now().as_secs();
    let v1 = build_draft_at(
        &owner,
        &d,
        "9",
        &ch_a,
        &fake_nip44_v2(),
        Timestamp::from(now - 1),
    );
    let (ok1, msg1) = submit_event_http(&client, &owner, &v1).await;
    assert!(ok1, "initial draft to ch_a must be accepted: {msg1}");

    // Attempt to update the same d to a different channel.
    let v2 = build_draft(&owner, &d, "9", &ch_b, &fake_nip44_v2());
    let (accepted, msg) = submit_event_http(&client, &owner, &v2).await;
    assert!(
        !accepted,
        "rebinding draft to a different channel must be rejected"
    );
    assert!(
        msg.contains("immutable") || msg.contains("channel"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_same_channel_replacement_accepted() {
    // Updating a draft on the same channel must succeed (normal NIP-33 replacement).
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;

    let d = uuid::Uuid::new_v4().to_string();
    let now = Timestamp::now().as_secs();
    let v1 = build_draft_at(
        &owner,
        &d,
        "9",
        &ch_id,
        &fake_nip44_v2(),
        Timestamp::from(now - 1),
    );
    let (ok1, msg1) = submit_event_http(&client, &owner, &v1).await;
    assert!(ok1, "v1 must be accepted: {msg1}");

    let v2 = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let v2_id = v2.id;
    let (ok2, msg2) = submit_event_http(&client, &owner, &v2).await;
    assert!(ok2, "v2 same-channel replacement must be accepted: {msg2}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(results.len(), 1, "replacement must leave exactly one head");
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        v2_id.to_hex(),
        "v2 must be the current head"
    );
}

// ─── Ingest validation (structural) ──────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_accepted_with_ciphertext_content() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(accepted, "valid draft rejected: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_accepted_blank_tombstone() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = build_tombstone(&owner, &d, "9", &ch_id, Timestamp::now());
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(accepted, "blank tombstone rejected: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_accepted_future_expiration() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
            Tag::parse(["expiration", "4102444800"]).unwrap(), // year 2100
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(accepted, "future expiration draft rejected: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_missing_d_tag() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "missing d tag should be rejected");
    assert!(msg.contains("d` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_empty_d_tag() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", ""]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "empty d tag should be rejected");
    assert!(msg.contains("d` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_oversized_d_tag() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    // D_TAG_MAX_LEN is 1024 bytes in buzz-db. Use 1025 'a' chars.
    let d_tag = "a".repeat(1025);
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d_tag]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "oversized d tag should be rejected");
    assert!(
        msg.contains("d` tag") || msg.contains("too long"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_duplicate_d_tag() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "duplicate d tag should be rejected");
    assert!(msg.contains("d` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_missing_k_tag() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "missing k tag should be rejected");
    assert!(msg.contains("k` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_duplicate_k_tag() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "duplicate k tag should be rejected");
    assert!(msg.contains("k` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_malformed_k_tag_non_decimal() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "0x9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "non-decimal k tag should be rejected");
    assert!(
        msg.contains("canonical decimal"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_k_tag_leading_zero() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "09"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "k tag with leading zero should be rejected");
    assert!(msg.contains("leading zero"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_k_tag_out_of_range() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "65536"]).unwrap(), // u16::MAX + 1
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "k=65536 should be rejected (out of u16 range)");
    assert!(msg.contains("range"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_p_tag() {
    let client = http_client();
    let owner = Keys::generate();
    // Use a different pubkey for the `p` tag — EventBuilder silently strips
    // `p` tags that match the signer's own key (NIP self-tagging rule), so
    // testing with owner.public_key() would produce an event with NO `p` tag
    // and the rejection would never be exercised.
    let other = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
            Tag::parse(["p", &other.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "p tag on draft should be rejected");
    assert!(msg.contains("p` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_malformed_ciphertext() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), "not-a-ciphertext")
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "malformed ciphertext should be rejected");
    assert!(
        msg.contains("base64") || msg.contains("NIP-44"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_expiration_in_past() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
            Tag::parse(["expiration", "1000000000"]).unwrap(), // long past
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "past expiration should be rejected");
    assert!(msg.contains("expiration"), "unexpected message: {msg}");
}

// ─── NIP-01 replacement / tombstone ordering ─────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_replaced_by_newer_event() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let now = Timestamp::now().as_secs();
    let t0 = Timestamp::from(now - 2);
    let t1 = Timestamp::from(now - 1);

    let v1 = build_draft_at(&owner, &d, "9", &ch_id, &fake_nip44_v2(), t0);
    let v2 = build_draft_at(&owner, &d, "9", &ch_id, &fake_nip44_v2(), t1);
    let v2_id = v2.id;

    let (ok1, msg1) = submit_event_http(&client, &owner, &v1).await;
    assert!(ok1, "v1 must be accepted: {msg1}");
    let (ok2, msg2) = submit_event_http(&client, &owner, &v2).await;
    assert!(ok2, "v2 must be accepted: {msg2}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(results.len(), 1, "should return exactly the latest draft");
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        v2_id.to_hex(),
        "latest event must be the returned head"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_stale_write_cannot_supersede_current_head() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let now = Timestamp::now().as_secs();
    let t_old = Timestamp::from(now - 2);
    let t_new = Timestamp::from(now - 1);

    let v_new = build_draft_at(&owner, &d, "9", &ch_id, &fake_nip44_v2(), t_new);
    let v_old = build_draft_at(&owner, &d, "9", &ch_id, &fake_nip44_v2(), t_old);

    let (ok_n, msg_n) = submit_event_http(&client, &owner, &v_new).await;
    assert!(ok_n, "newer draft must be accepted: {msg_n}");
    // The stale write must be accepted (relay returns `accepted: true` with
    // `duplicate:` or silently deduplicated) but MUST NOT become the new head.
    // The relay's stale-ordering protection keeps the newer event as head.
    let (stale_accepted, _stale_msg) = submit_event_http(&client, &owner, &v_old).await;
    assert!(
        stale_accepted,
        "stale write must be accepted (no-op), not hard-rejected; got: {_stale_msg}"
    );

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(results.len(), 1, "should have exactly one head");
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        v_new.id.to_hex(),
        "stale write must not replace current head"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_same_second_tie_break_lower_id_wins() {
    // Two events at identical timestamps: NIP-01 tie-break retains the one
    // with the lexically lower event ID, regardless of submission order.
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let ts = Timestamp::now();

    // Sign candidates until we have at least two with distinct IDs. Add a
    // per-candidate unknown tag so each signing call produces a unique event
    // (different tag payload → different event hash → distinct IDs even if the
    // Schnorr nonce were deterministic).
    let mut candidates: Vec<nostr::Event> = Vec::new();
    for i in 0u32..20 {
        let e = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
            .tags([
                Tag::parse(["d", &d]).unwrap(),
                Tag::parse(["k", "9"]).unwrap(),
                Tag::parse(["h", &ch_id]).unwrap(),
                // Unique-per-candidate sentinel tag — forces distinct event hashes.
                Tag::parse(["_tiebreak", &i.to_string()]).unwrap(),
            ])
            .custom_created_at(ts)
            .sign_with_keys(&owner)
            .unwrap();
        candidates.push(e);
    }
    // Deduplicate by ID (should never trigger, but kept for safety).
    candidates.dedup_by_key(|e| e.id.to_hex());
    // The unique _tiebreak tag guarantees distinct event hashes — this must
    // always produce at least 2 distinct IDs.  A silent return here would
    // allow the test to pass without ever exercising the tie-break logic.
    assert!(
        candidates.len() >= 2,
        "expected at least 2 distinct candidate IDs with unique _tiebreak tags; got {}",
        candidates.len()
    );
    candidates.sort_by_key(|a| a.id.to_hex());
    let lowest = candidates.first().unwrap().clone();
    let highest = candidates.last().unwrap().clone();

    // Submit highest first, then lowest.
    let (ok_h, msg_h) = submit_event_http(&client, &owner, &highest).await;
    assert!(ok_h, "highest-id draft must be accepted: {msg_h}");
    let (ok_l, msg_l) = submit_event_http(&client, &owner, &lowest).await;
    assert!(ok_l, "lowest-id draft must be accepted: {msg_l}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(results.len(), 1, "tie-break must leave exactly one head");
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        lowest.id.to_hex(),
        "lower event ID must win same-second tie"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_tombstone_head_queryable_by_author() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let now = Timestamp::now().as_secs();
    let t_draft = Timestamp::from(now - 1);
    let t_tomb = Timestamp::now();

    let draft = build_draft_at(&owner, &d, "9", &ch_id, &fake_nip44_v2(), t_draft);
    let tombstone = build_tombstone(&owner, &d, "9", &ch_id, t_tomb);
    let tomb_id = tombstone.id;

    let (ok_d, msg_d) = submit_event_http(&client, &owner, &draft).await;
    assert!(ok_d, "draft must be accepted: {msg_d}");
    let (ok_t, msg_t) = submit_event_http(&client, &owner, &tombstone).await;
    assert!(ok_t, "tombstone must be accepted: {msg_t}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(results.len(), 1, "tombstone must be the queryable head");
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        tomb_id.to_hex(),
        "tombstone is the current head"
    );
    assert_eq!(
        results[0]["content"].as_str().unwrap(),
        "",
        "tombstone content must be empty"
    );
}

// ─── Author-only read gates ───────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_author_can_req_own_drafts_ws() {
    let url = relay_url();
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok, msg) = submit_event_http(&client, &owner, &draft).await;
    assert!(ok, "draft must be accepted: {msg}");

    let mut c = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect author");
    let sid = sub_id("author-req");
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key());
    c.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let results = c
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");
    assert!(
        results.iter().any(|e| e.id == draft_id),
        "author must receive own draft"
    );
    c.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_req_victims_drafts_exclusive_ws() {
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("attacker-excl");
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key());
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");

    let relay_msg = ac
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv response");
    match relay_msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(subscription_id, sid);
            assert!(
                message.contains("restricted:") || message.contains("author-only"),
                "expected restricted message, got: {message}"
            );
        }
        RelayMessage::Event { event, .. } => {
            panic!(
                "attacker received victim's draft via exclusive filter: event {}",
                event.id
            );
        }
        other => panic!("expected CLOSED for exclusive draft filter, got: {other:?}"),
    }
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_see_draft_in_mixed_kinds_filter_ws() {
    // A filter with kinds=[0,31234] must return the victim's public profile (kind:0)
    // but MUST NOT return their draft (kind:31234).
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    // Victim publishes a public profile (kind:0).
    let profile = EventBuilder::new(Kind::Metadata, "{}")
        .sign_with_keys(&victim)
        .unwrap();
    let profile_id = profile.id;
    let (ok_p, msg_p) = submit_event_http(&client, &victim, &profile).await;
    assert!(ok_p, "victim profile must be accepted: {msg_p}");

    // Victim publishes a draft.
    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok_d, msg_d) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok_d, "victim draft must be accepted: {msg_d}");

    // Attacker subscribes with an explicit kinds=[0,31234] filter.
    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("mixed-kinds-0-31234");
    let filter = Filter::new()
        .kinds(vec![Kind::Metadata, Kind::Custom(KIND_DRAFT)])
        .author(victim.public_key());
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let results = ac
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert!(
        results.iter().any(|e| e.id == profile_id),
        "attacker must receive victim's public profile (positive control)"
    );
    assert!(
        !results.iter().any(|e| e.id == draft_id),
        "kinds=[0,31234] filter must not expose victim's draft to attacker"
    );
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_see_draft_in_mixed_longform_kinds_filter_ws() {
    // A filter with kinds=[30023,31234] must return the victim's long-form note
    // but MUST NOT return their draft (kind:31234).
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d_draft = uuid::Uuid::new_v4().to_string();

    // Victim publishes a long-form note (kind:30023 — global replaceable, public).
    let d_article = uuid::Uuid::new_v4().to_string();
    let article = EventBuilder::new(Kind::Custom(30023), "article content")
        .tags([Tag::parse(["d", &d_article]).unwrap()])
        .sign_with_keys(&victim)
        .unwrap();
    let article_id = article.id;
    let (ok_a, msg_a) = submit_event_http(&client, &victim, &article).await;
    assert!(ok_a, "victim article must be accepted: {msg_a}");

    // Victim publishes a draft.
    let draft = build_draft(&victim, &d_draft, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok_d, msg_d) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok_d, "victim draft must be accepted: {msg_d}");

    // Attacker subscribes with kinds=[30023,31234].
    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("mixed-kinds-30023-31234");
    let filter = Filter::new()
        .kinds(vec![Kind::Custom(30023), Kind::Custom(KIND_DRAFT)])
        .author(victim.public_key());
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let results = ac
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert!(
        results.iter().any(|e| e.id == article_id),
        "attacker must receive victim's public article (positive control)"
    );
    assert!(
        !results.iter().any(|e| e.id == draft_id),
        "kinds=[30023,31234] filter must not expose victim's draft to attacker"
    );
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_retrieve_by_known_event_id_ws() {
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("attacker-ids");
    let filter = Filter::new().id(draft_id);
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let results = ac
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");
    assert!(
        !results.iter().any(|e| e.id == draft_id),
        "knowing a draft's event id must not expose it to another user"
    );
    ac.disconnect().await.expect("disconnect");
}

// ─── known-#d privacy tripwires ───────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_retrieve_by_known_d_tag_exclusive_ws() {
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("d-excl");
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");

    let relay_msg = ac
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv response");
    match relay_msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(subscription_id, sid);
            assert!(
                message.contains("restricted:") || message.contains("author-only"),
                "expected restricted message for #d exclusive filter, got: {message}"
            );
        }
        RelayMessage::Event { event, .. } => {
            panic!(
                "attacker retrieved victim's draft via exclusive #d filter: event {}",
                event.id
            );
        }
        other => panic!("expected CLOSED for #d exclusive filter, got: {other:?}"),
    }
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_retrieve_draft_by_d_tag_in_mixed_kinds_ws() {
    // An attacker who knows the victim's d-tag value submits
    // kinds=[31234] + #d=[d_value] + author=[victim].  Must get CLOSED, not the event.
    //
    // Positive control: a public kind:30023 (long-form article) published under
    // the SAME `d` must be returned by kinds=[30023,31234]+author+#d — proving
    // the filter itself is not broken, only the draft is gated.
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    // Use the same `d` value for both the draft AND the kind:30023 control.
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    // Publish a public kind:30023 with the same d-tag — this is the positive
    // control that proves kinds=[30023,31234]+#d is a live filter, not a no-op.
    let article = EventBuilder::new(Kind::Custom(30023), "long-form article content")
        .tags([Tag::parse(["d", &d]).unwrap()])
        .sign_with_keys(&victim)
        .unwrap();
    let article_id = article.id;
    let (ok_a, msg_a) = submit_event_http(&client, &victim, &article).await;
    assert!(ok_a, "victim article must be accepted: {msg_a}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("d-mixed-31234");
    // kinds=[31234] + #d=[d] is the sharpest possible known-address query.
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");

    let relay_msg = ac
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv response");
    match relay_msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(subscription_id, sid);
            assert!(
                message.contains("restricted:") || message.contains("author-only"),
                "expected restricted message for #d+kind:31234 filter, got: {message}"
            );
        }
        RelayMessage::Event { event, .. } => {
            panic!(
                "attacker retrieved victim's draft via #d+kind:31234 filter: event {}",
                event.id
            );
        }
        other => panic!("expected CLOSED for #d+kind:31234 filter, got: {other:?}"),
    }

    // Mixed-kinds positive control: kinds=[30023,31234] + author + #d=[same d].
    // The kind:30023 article must appear; the kind:31234 draft must NOT.
    // Using explicit kinds avoids the p-gated wildcard guard.
    let sid2 = sub_id("d-mixed-control");
    let filter2 = Filter::new()
        .kinds(vec![Kind::Custom(30023), Kind::Custom(KIND_DRAFT)])
        .author(victim.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    ac.subscribe(&sid2, vec![filter2])
        .await
        .expect("subscribe mixed kinds");
    let mixed_results = ac
        .collect_until_eose(&sid2, Duration::from_secs(5))
        .await
        .expect("collect mixed kinds");
    assert!(
        mixed_results.iter().any(|e| e.id == article_id),
        "kind:30023 article under same d must appear in [30023,31234]+#d filter (positive control)"
    );
    assert!(
        !mixed_results.iter().any(|e| e.id == draft_id),
        "kind:31234 draft must not appear in [30023,31234]+#d filter for attacker"
    );

    ac.disconnect().await.expect("disconnect");
}

// ─── COUNT privacy gates ──────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_count_exclusive_ws() {
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("count-ws");
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key());
    ac.send_raw(&json!(["COUNT", sid, filter]))
        .await
        .expect("send COUNT");

    let relay_msg = ac
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv response");
    match relay_msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(subscription_id, sid);
            assert!(
                message.contains("restricted:") || message.contains("author-only"),
                "expected restricted message for COUNT on another author's drafts, got: {message}"
            );
        }
        other => {
            panic!("expected CLOSED for WS COUNT on another author's drafts, got: {other:?}")
        }
    }
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_count_via_known_d_ws() {
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("count-ws-d");
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    ac.send_raw(&json!(["COUNT", sid, filter]))
        .await
        .expect("send COUNT");

    let relay_msg = ac
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv response");
    match relay_msg {
        RelayMessage::Closed { message, .. } => {
            assert!(
                message.contains("restricted:") || message.contains("author-only"),
                "expected restricted for #d COUNT, got: {message}"
            );
        }
        other => panic!("expected CLOSED for #d COUNT, got: {other:?}"),
    }
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_count_exclusive_http() {
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key());
    let resp = client
        .post(format!("{}/count", relay_http_url()))
        .header("X-Pubkey", &attacker.public_key().to_hex())
        .header("Content-Type", "application/json")
        .json(&vec![filter])
        .send()
        .await
        .expect("count request");
    assert_eq!(
        resp.status().as_u16(),
        403,
        "HTTP exclusive COUNT for another author's drafts must return 403"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_author_can_count_own_drafts_http() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &owner, &draft).await;
    assert!(ok, "draft must be accepted: {msg}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key());
    let resp = client
        .post(format!("{}/count", relay_http_url()))
        .header("X-Pubkey", &owner.public_key().to_hex())
        .header("Content-Type", "application/json")
        .json(&vec![filter])
        .send()
        .await
        .expect("count request");
    assert!(
        resp.status().is_success(),
        "author's own count must succeed, got: {}",
        resp.status()
    );
    let body: Value = resp.json().await.expect("parse count response");
    let count = body["count"].as_u64().unwrap_or(0);
    assert!(count >= 1, "author must count at least 1 own draft");
}

// ─── HTTP /query exclusive-author privacy ────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_query_exclusive_http() {
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key());
    let resp = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", &attacker.public_key().to_hex())
        .header("Content-Type", "application/json")
        .json(&vec![filter])
        .send()
        .await
        .expect("query request");
    assert_eq!(
        resp.status().as_u16(),
        403,
        "exclusive other-author HTTP /query for kind:31234 must return 403"
    );
}

// ─── Live fan-out privacy ─────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_live_fanout_only_reaches_author() {
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid_fanout = sub_id("fanout-attacker");
    let filter = Filter::new()
        .kinds(vec![Kind::Metadata, Kind::Custom(KIND_DRAFT)])
        .author(victim.public_key())
        .limit(0);
    ac.subscribe(&sid_fanout, vec![filter])
        .await
        .expect("subscribe to mixed filter");
    let _ = ac
        .collect_until_eose(&sid_fanout, Duration::from_secs(3))
        .await;

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok_d, msg_d) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok_d, "draft must be accepted: {msg_d}");

    let profile = EventBuilder::new(Kind::Metadata, "{}")
        .sign_with_keys(&victim)
        .unwrap();
    let profile_id = profile.id;
    let (ok_p, msg_p) = submit_event_http(&client, &victim, &profile).await;
    assert!(ok_p, "profile must be accepted: {msg_p}");

    let mut received_draft = false;
    let mut received_profile = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or(Duration::ZERO);
        if remaining.is_zero() {
            break;
        }
        match ac.recv_event(remaining).await {
            Ok(RelayMessage::Event { event, .. }) => {
                if event.id == draft_id {
                    received_draft = true;
                }
                if event.id == profile_id {
                    received_profile = true;
                }
            }
            _ => break,
        }
    }

    assert!(
        !received_draft,
        "attacker must NOT receive victim's draft via live fan-out"
    );
    assert!(
        received_profile,
        "attacker MUST receive victim's public profile (positive control)"
    );
    ac.disconnect().await.expect("disconnect");
}

// ─── Nonexistent / alien channel rejection ────────────────────────────────────

// NOTE: test_draft_rejected_nonexistent_channel_h_tag (at the top of this file)
// already covers this: a draft with a valid-UUID h-tag pointing to no live
// channel is rejected.  That test is the single authoritative nonexistent-channel
// guard.  True cross-community tenant confinement is covered at the DB layer by
// `draft_is_confined_to_its_community` (requires Postgres, wired to CI).

// ─── Kindless channel query — draft privacy ───────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_not_returned_in_kindless_channel_query_by_attacker() {
    // A kindless channel h-tag filter submitted by a non-author must never
    // return the author's draft.  The attacker has channel membership (the
    // channel is open) and subscribes to all events in the channel — they
    // must receive the owner's public messages but not their drafts.
    let url = relay_url();
    let client = http_client();
    let owner = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok, msg) = submit_event_http(&client, &owner, &draft).await;
    assert!(ok, "draft must be accepted: {msg}");

    // Also publish a public channel message as a positive control.
    let msg_event = EventBuilder::new(Kind::Custom(9), "hello channel")
        .tags([Tag::parse(["h", &ch_id]).unwrap()])
        .sign_with_keys(&owner)
        .unwrap();
    let msg_id = msg_event.id;
    let (ok_m, msg_m) = submit_event_http(&client, &owner, &msg_event).await;
    assert!(ok_m, "channel message must be accepted: {msg_m}");

    // Attacker queries by channel h-tag, no kind filter.
    let mut c = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("ch-kindless-attacker");
    let filter = Filter::new().custom_tag(
        nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
        ch_id.as_str(),
    );
    c.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let results = c
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    // Channel message must appear (positive control — attacker can see public messages).
    assert!(
        results.iter().any(|e| e.id == msg_id),
        "channel message must appear in attacker's h-tag query (positive control)"
    );
    // Draft must be absent — author-only gate must strip it before delivery.
    assert!(
        !results.iter().any(|e| e.id == draft_id),
        "draft must not be returned by a kindless channel h-tag filter to a non-author"
    );
    c.disconnect().await.expect("disconnect");
}

// ─── FTS / NIP-50 exclusion ───────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_not_indexed_in_fts_search() {
    // A kind:31234 has NULL search_tsv at the storage layer, so NIP-50 search
    // must never surface it — even when the requester is the author.
    //
    // The test explicitly uses kinds=[1,31234] in the search filter so the query
    // cannot be satisfied by the read-gate alone: if kind:31234 had a non-NULL
    // tsvector matching the token, the relay would return it to the authorized
    // author.  NULL tsvector is the only thing that hides it.
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    // Unique word token — used as kind:1 content (FTS-indexed) and as the
    // search query for both kinds.
    let token = format!("nip37probe{}", uuid::Uuid::new_v4().simple());

    // Kind:1 control note — MUST appear in FTS results.
    let note = EventBuilder::new(Kind::TextNote, &token)
        .sign_with_keys(&victim)
        .unwrap();
    let note_id = note.id;
    let (ok_note, msg_note) = submit_event_http(&client, &victim, &note).await;
    assert!(ok_note, "control note must be accepted: {msg_note}");

    // Kind:31234 draft — NIP-44 v2 content (relay validates).  Storage migration
    // sets search_tsv = NULL for all kind:31234 rows, so even a theoretically
    // searchable payload must not surface in FTS results.
    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok_d, msg_d) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok_d, "draft must be accepted: {msg_d}");

    // Search as the author with explicit kinds=[1,31234].  The kind:31234 draft
    // is excluded by NULL search_tsv; the kind:1 note IS found.
    let search_filter = Filter::new()
        .kinds(vec![Kind::TextNote, Kind::Custom(KIND_DRAFT)])
        .search(&token)
        .limit(50);
    let results =
        query_events_http(&client, &victim.public_key().to_hex(), vec![search_filter]).await;

    assert!(
        results
            .iter()
            .any(|e| e["id"].as_str() == Some(&note_id.to_hex())),
        "FTS must index the control kind:1 note (positive control)"
    );
    assert!(
        !results
            .iter()
            .any(|e| e["id"].as_str() == Some(&draft_id.to_hex())),
        "kind:31234 must have NULL search_tsv — draft must not appear in NIP-50 search"
    );

    // Attacker-side check: search with kinds=[1,31234] as attacker.
    let attacker_filter = Filter::new()
        .kinds(vec![Kind::TextNote, Kind::Custom(KIND_DRAFT)])
        .search(&token)
        .limit(50);
    let attacker_results = query_events_http(
        &client,
        &attacker.public_key().to_hex(),
        vec![attacker_filter],
    )
    .await;
    assert!(
        !attacker_results
            .iter()
            .any(|e| e["id"].as_str() == Some(&draft_id.to_hex())),
        "draft must not appear in attacker's NIP-50 search either"
    );
}

// ─── NIP-11 advertisement ─────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_nip11_advertises_nip37_not_nip40() {
    let client = http_client();
    let resp = client
        .get(relay_http_url())
        .header("Accept", "application/nostr+json")
        .send()
        .await
        .expect("NIP-11 request");
    assert!(resp.status().is_success());
    let info: Value = resp.json().await.expect("parse NIP-11 response");
    let nips = info["supported_nips"]
        .as_array()
        .expect("supported_nips must be an array");
    let nip_numbers: Vec<u64> = nips.iter().filter_map(|v| v.as_u64()).collect();
    assert!(
        nip_numbers.contains(&37),
        "NIP-11 must advertise NIP-37 (draft wraps); got {nip_numbers:?}"
    );
    assert!(
        !nip_numbers.contains(&40),
        "NIP-11 must NOT advertise NIP-40 (expiry suppression not implemented); got {nip_numbers:?}"
    );
}

// ─── NIP-09 a-tag deletion guard ─────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_nip09_a_tag_deletion_of_draft_is_rejected() {
    // kind:5 with a single `a` tag targeting `31234:<pubkey>:<d>` must be
    // rejected at ingest.  The relay must never let a kind:5 event act as an
    // escape hatch to clear a draft's immutable channel binding.
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    // First publish a draft so there is something to attempt to delete.
    let draft = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok_d, msg_d) = submit_event_http(&client, &owner, &draft).await;
    assert!(
        ok_d,
        "draft must be accepted before the deletion attempt: {msg_d}"
    );

    // Build kind:5 with a single a-tag targeting the draft's NIP-33 address.
    let a_coord = format!("31234:{}:{}", owner.public_key().to_hex(), d);
    let deletion = EventBuilder::new(Kind::EventDeletion, "")
        .tags([Tag::parse(["a", &a_coord]).unwrap()])
        .sign_with_keys(&owner)
        .unwrap();

    let (accepted, msg) = submit_event_http(&client, &owner, &deletion).await;
    assert!(
        !accepted,
        "kind:5 a-tag deletion targeting kind:31234 must be rejected; relay said: {msg}"
    );
    assert!(
        msg.contains("31234")
            || msg.contains("draft")
            || msg.contains("not supported")
            || msg.contains("invalid"),
        "rejection message must explain why; got: {msg}"
    );

    // The draft must still exist as a live head — the rejected kind:5 must not
    // have modified anything.
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(
        results.len(),
        1,
        "draft must still have one live head after rejected kind:5 deletion"
    );
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        draft.id.to_hex(),
        "live head must still be the original draft — kind:5 must not have altered it"
    );
}

// ─── DM channel path ─────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_accepted_and_replaced_in_dm_channel() {
    // Draft wraps are channel-bound. A DM channel UUID (returned from
    // kind:41010) is a valid `h` target — the relay treats it identically
    // to a stream/broadcast channel for draft storage purposes.
    let client = http_client();
    let alice = Keys::generate();
    let bob = Keys::generate();

    // Alice opens a DM with Bob — relay creates and returns the channel UUID.
    let dm_event = EventBuilder::new(Kind::Custom(41010), "")
        .tags([Tag::parse(["p", &bob.public_key().to_hex()]).unwrap()])
        .sign_with_keys(&alice)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &alice.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&dm_event).unwrap())
        .send()
        .await
        .expect("open DM");
    let body: Value = resp.json().await.expect("parse DM response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "DM open must be accepted: {body}"
    );
    // The relay embeds the DM channel UUID in the message payload.
    let msg = body["message"].as_str().unwrap_or("");
    let dm_channel_id = if let Some(stripped) = msg.strip_prefix("response:") {
        let parsed: Value = serde_json::from_str(stripped).expect("response JSON");
        parsed["channel_id"]
            .as_str()
            .expect("channel_id in DM response")
            .to_string()
    } else {
        panic!(
            "DM open response must contain a `response:{{...}}` payload with channel_id; \
             got message: {msg:?} (full body: {body})"
        );
    };

    // Alice submits a draft bound to the DM channel UUID.
    // Timestamps are strictly increasing to guarantee deterministic ordering.
    let d = uuid::Uuid::new_v4().to_string();
    let base = nostr::Timestamp::now().as_secs();
    let v1 = build_draft_at(
        &alice,
        &d,
        "9",
        &dm_channel_id,
        &fake_nip44_v2(),
        nostr::Timestamp::from(base - 2),
    );
    let (ok1, msg1) = submit_event_http(&client, &alice, &v1).await;
    assert!(ok1, "draft v1 to DM channel must be accepted: {msg1}");

    // Replace with a strictly newer version (base - 1 > base - 2).
    let v2 = build_draft_at(
        &alice,
        &d,
        "9",
        &dm_channel_id,
        &fake_nip44_v2(),
        nostr::Timestamp::from(base - 1),
    );
    let v2_id = v2.id;
    let (ok2, msg2) = submit_event_http(&client, &alice, &v2).await;
    assert!(
        ok2,
        "draft v2 replacement in DM channel must be accepted: {msg2}"
    );

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(alice.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &alice.public_key().to_hex(), vec![filter]).await;
    assert_eq!(
        results.len(),
        1,
        "DM-channel draft must have exactly one head"
    );
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        v2_id.to_hex(),
        "v2 must be the head after replacement"
    );

    // Tombstone the draft (base > base - 1, so this supersedes v2).
    let tomb = build_tombstone(
        &alice,
        &d,
        "9",
        &dm_channel_id,
        nostr::Timestamp::from(base),
    );
    let tomb_id = tomb.id;
    let (ok_t, msg_t) = submit_event_http(&client, &alice, &tomb).await;
    assert!(ok_t, "tombstone in DM channel must be accepted: {msg_t}");

    let filter2 = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(alice.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results2 = query_events_http(&client, &alice.public_key().to_hex(), vec![filter2]).await;
    assert_eq!(results2.len(), 1, "tombstone must be the only head");
    assert_eq!(
        results2[0]["id"].as_str().unwrap(),
        tomb_id.to_hex(),
        "tombstone must be the current head after DM-channel draft is closed"
    );
}

// ─── Removed-member read denial ──────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_removed_member_cannot_read_drafts_after_removal() {
    // A member who wrote a draft is later removed from the channel.
    // After removal they must not be able to REQ or COUNT their own draft
    // (channel membership is required for draft reads, not just writes).
    //
    // Note: draft reads are author-only, so this also tests that the
    // author-only gate stacks correctly with the channel-membership gate.
    let url = relay_url();
    let client = http_client();
    let owner = Keys::generate();
    let member = Keys::generate();
    let ch_id = create_private_channel(&owner).await;
    add_member_http(&client, &owner, &ch_id, &member).await;

    let d = uuid::Uuid::new_v4().to_string();
    let v1 = build_draft_at(
        &member,
        &d,
        "9",
        &ch_id,
        &fake_nip44_v2(),
        nostr::Timestamp::from(nostr::Timestamp::now().as_secs() - 2),
    );
    let (ok1, msg1) = submit_event_http(&client, &member, &v1).await;
    assert!(ok1, "member draft must be accepted: {msg1}");
    let v1_id = v1.id;

    // Remove member.
    remove_member_http(&client, &owner, &ch_id, &member).await;

    // Historical REQ: removed member must not retrieve their old draft.
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(member.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results =
        query_events_http(&client, &member.public_key().to_hex(), vec![filter.clone()]).await;
    // The relay may return CLOSED or 0 results; the draft must not appear.
    assert!(
        !results
            .iter()
            .any(|e| e["id"].as_str() == Some(&v1_id.to_hex())),
        "removed member must not retrieve their draft via HTTP /query"
    );

    // HTTP COUNT: removed member must get 0 count (not 403 here, since it's
    // their own draft address, but the channel-membership check should exclude it).
    let count_resp = client
        .post(format!("{}/count", relay_http_url()))
        .header("X-Pubkey", &member.public_key().to_hex())
        .header("Content-Type", "application/json")
        .json(&vec![filter])
        .send()
        .await
        .expect("count request");
    let count_status = count_resp.status().as_u16();
    if count_status == 200 {
        let body: Value = count_resp.json().await.expect("parse count");
        let count = body["count"].as_u64().unwrap_or(0);
        assert_eq!(
            count, 0,
            "removed member's draft count must be 0 after removal"
        );
    } else {
        // 403 is also acceptable — relay may gate based on membership entirely.
        assert!(
            count_status == 403,
            "expected 200 with count=0 or 403, got {count_status}"
        );
    }

    // Live fan-out: owner posts a new draft to the channel.  The removed
    // member must not receive it via a pre-existing WS subscription, even
    // if they filter on author(owner) — only current channel members may
    // receive owner's drafts.
    let mut removed_client = BuzzTestClient::connect(&url, &member)
        .await
        .expect("connect removed member");
    let sid = sub_id("removed-fanout");
    // Subscribe to the owner's drafts — if channel membership is properly
    // enforced, the relay must CLOSE or simply not deliver owner's new draft
    // to the removed member.
    let live_filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .limit(0); // skip historical; only live
    let _ = removed_client.subscribe(&sid, vec![live_filter]).await;
    let _ = removed_client
        .collect_until_eose(&sid, Duration::from_secs(2))
        .await;

    // Owner submits a new draft — this is the live probe event.
    let owner_draft = build_draft(
        &owner,
        &uuid::Uuid::new_v4().to_string(),
        "9",
        &ch_id,
        &fake_nip44_v2(),
    );
    let owner_draft_id = owner_draft.id;
    let (ok_od, _) = submit_event_http(&client, &owner, &owner_draft).await;
    // Owner's own draft must be accepted; verify it doesn't reach removed member.
    if ok_od {
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while let Ok(RelayMessage::Event { event, .. }) =
                removed_client.recv_event(Duration::from_secs(1)).await
            {
                if event.id == owner_draft_id {
                    panic!("removed member received owner's draft via live fan-out after removal");
                }
            }
        })
        .await;
    }
    removed_client.disconnect().await.expect("disconnect");
}
