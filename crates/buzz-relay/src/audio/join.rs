//! Cross-pod huddle join coordination over the mesh `HuddleControl` profile.
//!
//! This is the control plane that decides, at join time, *which pod owns a
//! huddle* and wires a client into that owner's room — the counterpart to the
//! media datagram fan-out in [`super::mesh`]. It is the join path
//! [`super::handler`] calls once a client has authed and passed membership.
//!
//! ## Ownership decision (Redis is the arbiter, mesh is only a hint)
//!
//! A huddle's `session_id` is its `channel_id`. Exactly one pod owns it: the
//! holder of the Redis fenced CAS lease. On join we resolve ownership through
//! the [`HuddleDirectory`]:
//!
//! - **No live lease** → this pod acquires it and becomes owner
//!   ([`JoinOutcome::LocalOwner`]). The client admits to a local [`Room`] as in
//!   a single-pod huddle.
//! - **Lease held by us** → [`JoinOutcome::LocalOwner`] at the live generation.
//! - **Lease held by another pod** → [`JoinOutcome::RemoteOwner`]. The client
//!   admits to a *local* room too, but the pod also opens a `HuddleControl`
//!   stream to the owner and registers the client as a remote peer there so the
//!   owner fans media back (see [`super::mesh`]).
//!
//! Membership never grants ownership: it may say "route to that pod," never
//! "take over." The owner side re-validates every registration's fence against
//! Redis on receipt — fencing at every hop, not just at the origin — so a lease
//! that changes between our lookup and the owner's receipt is caught there.
//!
//! ## `HuddleControl` payload schema (owned here)
//!
//! The mesh wire layer carries huddle-control bytes opaquely in
//! [`MeshStreamFrame::Data`](buzz_relay_mesh::MeshStreamFrame). Their layout is
//! [`HuddleControlMsg`], postcard-encoded. Non-owner → owner:
//! [`HuddleControlMsg::RegisterPeer`] / [`HuddleControlMsg::UnregisterPeer`].
//! Owner → non-owner: [`HuddleControlMsg::PeerRegistered`] (assigned index) or
//! [`HuddleControlMsg::RegisterRejected`] (fence/admission failure surfaced to
//! the client as a join error, never a silent media drop).

use std::sync::Arc;

use buzz_core::CommunityId;
use buzz_relay_mesh::{
    FencedHeader, GoodbyeReason, MeshDatagram, MeshError, MeshStream, MeshStreamFrame, Profile,
    RelayPeerTransport, RuntimeId, StreamHello, StreamRole,
};
use serde::{Deserialize, Serialize};
use tracing::debug;
use uuid::Uuid;

use super::mesh::spawn_remote_peer_sink;
use super::room::{AdmissionError, AudioRoomManager};

/// The slice of the Redis fenced session directory the huddle join path needs.
///
/// Implemented by the session-directory lane's `SessionDirectory` over the
/// Redis CAS lease (see [`crate::tunnel::directory`]). Kept as a trait so the
/// coordinator is unit-testable without Redis, and so the handler depends on a
/// capability rather than a concrete type. Every method is a fenced,
/// linearizable Redis operation — the arbiter of ownership.
#[async_trait::async_trait]
pub trait HuddleDirectory: Send + Sync {
    /// Look up the live owner + generation for a huddle, or `None` if no lease
    /// exists yet.
    async fn owner_of(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
    ) -> Result<Option<Ownership>, MeshError>;

    /// Acquire ownership of a huddle if it is currently unowned. `owner` is the
    /// runtime that would own the lease (this pod's mesh identity). Returns the
    /// resulting ownership either way: `Acquired` when this pod took the lease,
    /// `Held` when another pod won the race (CAS lost).
    async fn acquire(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
        owner: RuntimeId,
    ) -> Result<AcquireOutcome, MeshError>;

    /// Validate a fenced header against the live lease. Returns a typed
    /// [`MeshError`] fence rejection when the frame is stale / unowned /
    /// owner-mismatched — the caller surfaces this to the client as a join
    /// rejection, never a media drop.
    async fn validate(
        &self,
        community_id: CommunityId,
        fenced: &FencedHeader,
    ) -> Result<(), MeshError>;
}

/// Bridges the concrete Redis-backed [`SessionDirectory`] to the huddle join
/// path's capability trait. Huddle sessions always use the
/// [`Profile::HuddleControl`] profile when acquiring a lease, so the profile is
/// fixed here rather than threaded through the join API.
#[async_trait::async_trait]
impl HuddleDirectory for crate::tunnel::directory::SessionDirectory {
    async fn owner_of(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
    ) -> Result<Option<Ownership>, MeshError> {
        let lease = self
            .lookup(community_id, session_id)
            .await
            .map_err(|e| MeshError::Transport(e.to_string()))?;
        Ok(lease.map(|l| Ownership {
            owner_runtime_id: l.owner_runtime_id,
            generation: l.generation,
        }))
    }

    async fn acquire(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
        owner: RuntimeId,
    ) -> Result<AcquireOutcome, MeshError> {
        use crate::tunnel::directory::AcquireResult;
        let result = self
            .acquire(community_id, session_id, owner, HUDDLE_CONTROL_PROFILE)
            .await
            .map_err(|e| MeshError::Transport(e.to_string()))?;
        Ok(match result {
            AcquireResult::Acquired(l) => AcquireOutcome::Acquired(Ownership {
                owner_runtime_id: l.owner_runtime_id,
                generation: l.generation,
            }),
            AcquireResult::Exists(l) => AcquireOutcome::Held(Ownership {
                owner_runtime_id: l.owner_runtime_id,
                generation: l.generation,
            }),
        })
    }

    async fn validate(
        &self,
        community_id: CommunityId,
        fenced: &FencedHeader,
    ) -> Result<(), MeshError> {
        self.validate_fenced_header(community_id, fenced).await
    }
}

/// A resolved ownership snapshot: which pod owns a huddle, at what generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ownership {
    /// Runtime holding the Redis lease for this huddle.
    pub owner_runtime_id: RuntimeId,
    /// Fenced generation of this ownership epoch; monotonic per session.
    pub generation: u64,
}

/// Result of an ownership acquire attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcquireOutcome {
    /// This pod created the lease and owns the returned generation.
    Acquired(Ownership),
    /// Another pod already holds the lease (CAS lost); route to it instead.
    Held(Ownership),
}

/// What the handler should do with a join, decided by ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinOutcome {
    /// This pod owns the huddle: admit the client to a local room directly, as
    /// in a single-pod huddle. Cross-pod peers (if any) reach us over the mesh.
    LocalOwner {
        /// Fenced generation this pod owns — stamped on media it fans out.
        generation: u64,
    },
    /// Another pod owns the huddle: admit the client locally *and* register it
    /// with the owner over `HuddleControl` so the owner fans media back. The
    /// fenced header is pre-validated against the live lease; the owner
    /// re-validates on receipt.
    RemoteOwner {
        /// Owner to open the `HuddleControl` stream to.
        owner_runtime_id: RuntimeId,
        /// Fenced generation of the owner's epoch.
        generation: u64,
    },
}

impl JoinOutcome {
    /// The fenced header for frames this join produces, given the huddle's
    /// session id (its channel id) and resolved owner. For a local-owner join
    /// the owner is this pod (`local_runtime_id`); for a remote-owner join it
    /// is the resolved owner.
    pub fn fenced_header(&self, session_id: Uuid, local_runtime_id: RuntimeId) -> FencedHeader {
        match *self {
            JoinOutcome::LocalOwner { generation } => FencedHeader {
                session_id,
                generation,
                owner_runtime_id: local_runtime_id,
            },
            JoinOutcome::RemoteOwner {
                owner_runtime_id,
                generation,
            } => FencedHeader {
                session_id,
                generation,
                owner_runtime_id,
            },
        }
    }
}

/// Resolve who owns a huddle and how this pod should join it.
///
/// The ownership plane is Redis-arbitrated: we look up the live lease and, only
/// if the huddle is unowned, attempt to acquire it (losing the CAS gracefully
/// hands us a `RemoteOwner` outcome pointing at the winner). A remote-owner
/// outcome is fence-validated against the live lease before we route to it, so
/// a caller never opens a control stream on a header Redis would reject.
///
/// `local_runtime_id` is this pod's mesh identity, used to tell "I own it" from
/// "someone else owns it."
pub async fn resolve_join<D: HuddleDirectory + ?Sized>(
    directory: &D,
    community_id: CommunityId,
    session_id: Uuid,
    local_runtime_id: RuntimeId,
) -> Result<JoinOutcome, MeshError> {
    // Look up the live lease first: the common steady-state case is an already
    // owned huddle, and we avoid an acquire attempt (and its generation INCR
    // race window) when a live owner already exists.
    let ownership = match directory.owner_of(community_id, session_id).await? {
        Some(o) => o,
        None => {
            // Unowned: try to take it. A lost CAS means a peer beat us to it
            // between our lookup and acquire — treat the winner as the owner.
            match directory
                .acquire(community_id, session_id, local_runtime_id)
                .await?
            {
                AcquireOutcome::Acquired(o) => {
                    return Ok(JoinOutcome::LocalOwner {
                        generation: o.generation,
                    });
                }
                AcquireOutcome::Held(o) => o,
            }
        }
    };

    if ownership.owner_runtime_id == local_runtime_id {
        return Ok(JoinOutcome::LocalOwner {
            generation: ownership.generation,
        });
    }

    // Remote owner: validate the fence against the live lease before we commit
    // to routing there. This is the origin-side hop of the fencing law; the
    // owner re-validates on receipt.
    let fenced = FencedHeader {
        session_id,
        generation: ownership.generation,
        owner_runtime_id: ownership.owner_runtime_id,
    };
    directory.validate(community_id, &fenced).await?;

    Ok(JoinOutcome::RemoteOwner {
        owner_runtime_id: ownership.owner_runtime_id,
        generation: ownership.generation,
    })
}

/// `HuddleControl` stream payload, carried in
/// [`MeshStreamFrame::Data`](buzz_relay_mesh::MeshStreamFrame)`.payload`,
/// postcard-encoded. This schema is owned by the huddle lane; the mesh wire
/// layer treats it as opaque bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HuddleControlMsg {
    /// Non-owner → owner: register a local client as a remote peer in the
    /// owner's room. The owner allocates the `peer_index`.
    RegisterPeer {
        /// Nostr pubkey hex of the joining client.
        pubkey: String,
        /// Huddle audio protocol version the client negotiated; the owner's
        /// room is pinned to one version and rejects mismatches.
        protocol_version: u8,
    },
    /// Owner → non-owner: the client is registered; here is its assigned index.
    PeerRegistered {
        /// Pubkey the registration was for (echoed for correlation).
        pubkey: String,
        /// Owner-allocated 0..=254 index; the sole allocator is the owner, so
        /// indices never collide across pods.
        peer_index: u8,
    },
    /// Owner → non-owner: registration refused. Surfaced to the client as a
    /// join error (e.g. `room_full`, `upgrade_required`, or a fence rejection),
    /// never a silent media drop.
    RegisterRejected {
        /// Pubkey the registration was for (echoed for correlation).
        pubkey: String,
        /// Machine-readable reason, matching the single-pod WS error `code`s
        /// where applicable (`room_full`, `room_ended`, `upgrade_required`) or
        /// a fence reason (`stale_generation`, `no_active_lease`,
        /// `owner_mismatch`, `future_generation`).
        reason: RegisterRejection,
    },
    /// Non-owner → owner: the local client left; drop its remote peer.
    UnregisterPeer {
        /// Pubkey of the departing client.
        pubkey: String,
    },
}

/// Why an owner refused a remote-peer registration. Mirrors the single-pod
/// admission failures plus the fence-rejection taxonomy, so a cross-pod join
/// surfaces the same client-facing error a same-pod join would.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegisterRejection {
    /// Owner's room hit the peer cap / exhausted the index space.
    RoomFull,
    /// Owner's room has ended (auto-ended or archived).
    RoomEnded,
    /// Owner's room is pinned to a different protocol version.
    VersionMismatch {
        /// Version the owner's room is pinned to.
        pinned: u8,
        /// Version the joining client requested.
        requested: u8,
    },
    /// The registration's fence was rejected by Redis on the owner. Carries the
    /// fence reason so `/_mesh` and the client see the same taxonomy the media
    /// path uses.
    Fenced(FenceRejection),
}

/// The Redis fence-rejection reasons, as a serializable enum for the
/// `HuddleControl` wire (the crate's [`MeshError`] fence variants are not
/// `Serialize`). Kept 1:1 with those variants so nothing is lost across the
/// wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FenceRejection {
    /// Frame generation is below the known floor.
    StaleGeneration,
    /// No live lease exists for the session.
    NoActiveLease,
    /// Frame owner does not match the live lease owner.
    OwnerMismatch,
    /// Frame generation does not match the live lease generation.
    FutureGeneration,
}

impl FenceRejection {
    /// Classify a [`MeshError`] fence variant. Returns `None` for non-fence
    /// errors (transport, encode/decode) — those are not registration
    /// rejections and are handled as stream failures by the caller.
    pub fn from_mesh_error(err: &MeshError) -> Option<Self> {
        match err {
            MeshError::StaleGeneration { .. } => Some(Self::StaleGeneration),
            MeshError::NoActiveLease { .. } => Some(Self::NoActiveLease),
            MeshError::OwnerMismatch { .. } => Some(Self::OwnerMismatch),
            MeshError::FutureGeneration { .. } => Some(Self::FutureGeneration),
            _ => None,
        }
    }

    /// Stable machine-readable code, matching the media path / `/_mesh`
    /// taxonomy (`stale_generation` | `no_active_lease` | `owner_mismatch` |
    /// `future_generation`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::StaleGeneration => "stale_generation",
            Self::NoActiveLease => "no_active_lease",
            Self::OwnerMismatch => "owner_mismatch",
            Self::FutureGeneration => "future_generation",
        }
    }
}

/// Encode a [`HuddleControlMsg`] for a `MeshStreamFrame::Data` payload.
pub fn encode_control(msg: &HuddleControlMsg) -> Result<Vec<u8>, MeshError> {
    postcard::to_allocvec(msg).map_err(MeshError::Encode)
}

/// Decode a `HuddleControl` `Data` payload back into a [`HuddleControlMsg`].
pub fn decode_control(bytes: &[u8]) -> Result<HuddleControlMsg, MeshError> {
    postcard::from_bytes(bytes).map_err(MeshError::Decode)
}

/// The tunnel profile these control messages ride. `HuddleControl` is a
/// reliable stream — a dropped roster delta is an unrecoverable peer-index
/// desync, so it never rides datagrams.
pub const HUDDLE_CONTROL_PROFILE: Profile = Profile::HuddleControl;

// ── Owner-side HuddleControl accept path ─────────────────────────────────────
//
// The owner pod hosts the real [`Room`]. When a *non-owner* pod opens a
// `HuddleControl` stream and registers a client, the owner admits that client
// as an ordinary [`AudioPeer`] whose `audio_tx` is drained by
// [`super::mesh::spawn_remote_peer_sink`] back to the non-owner pod as
// datagrams. `Room` never learns about the mesh — a remote participant looks
// exactly like a local one to fan-out.

/// Owner-side handler for inbound `HuddleControl` streams.
///
/// One instance per relay; the boot-seam dispatcher routes every
/// `Profile::HuddleControl` session stream to [`Self::accept_inbound`]. It is
/// the counterpart to Perci's reliable-stream acceptor — same
/// `accept_inbound(community_id, from, hello, stream)` shape, different profile
/// and body.
pub struct HuddleControlAcceptor<D: HuddleDirectory + ?Sized> {
    rooms: Arc<AudioRoomManager>,
    transport: Arc<dyn RelayPeerTransport>,
    directory: Arc<D>,
    local_runtime_id: RuntimeId,
}

impl<D: HuddleDirectory + ?Sized> HuddleControlAcceptor<D> {
    /// Build the acceptor. `directory` is the fenced arbiter (re-validated on
    /// every registration); `transport` is used to open the media datagram
    /// sink back to each registering pod.
    pub fn new(
        rooms: Arc<AudioRoomManager>,
        transport: Arc<dyn RelayPeerTransport>,
        directory: Arc<D>,
        local_runtime_id: RuntimeId,
    ) -> Self {
        Self {
            rooms,
            transport,
            directory,
            local_runtime_id,
        }
    }

    /// Accept and validate an inbound `HuddleControl` stream, then serve its
    /// register/unregister control loop until the stream closes.
    ///
    /// Validation mirrors the reliable-stream acceptor and enforces the fencing
    /// law on receipt: the claimed sender must be the authenticated peer, the
    /// profile must be `HuddleControl`, the fence must pass Redis, and this pod
    /// must be the fenced owner. Any of these failing rejects the stream before
    /// a single peer is admitted.
    pub async fn accept_inbound(
        &self,
        community_id: CommunityId,
        from: RuntimeId,
        hello: StreamHello,
        stream: MeshStream,
    ) -> Result<(), MeshError> {
        if hello.sender != from {
            return Err(MeshError::Transport(format!(
                "huddle-control hello.sender {} != authenticated peer {from}",
                hello.sender
            )));
        }
        let StreamRole::Session { fenced, profile } = hello.role else {
            return Err(MeshError::Transport(
                "huddle-control stream Hello was not a session role".into(),
            ));
        };
        if profile != Profile::HuddleControl {
            return Err(MeshError::Transport(format!(
                "huddle-control acceptor got profile {profile:?}"
            )));
        }
        // Fence at every hop: the origin validated before dialing; the owner
        // re-validates on receipt so a lease that moved in between is caught.
        self.directory.validate(community_id, &fenced).await?;
        if fenced.owner_runtime_id != self.local_runtime_id {
            return Err(MeshError::OwnerMismatch {
                session_id: fenced.session_id,
                generation: fenced.generation,
                frame_owner_runtime_id: fenced.owner_runtime_id,
                current_owner_runtime_id: self.local_runtime_id,
            });
        }

        self.serve_control_loop(community_id, from, fenced, stream)
            .await
    }

    /// Serve register/unregister frames for one non-owner pod's stream.
    ///
    /// Peers this stream registers are tracked so a stream close (the non-owner
    /// pod went away) tears them all down — no leaked remote peers holding
    /// index slots in the owner's room.
    async fn serve_control_loop(
        &self,
        community_id: CommunityId,
        from: RuntimeId,
        fenced: FencedHeader,
        mut stream: MeshStream,
    ) -> Result<(), MeshError> {
        let session_id = fenced.session_id;
        // pubkey -> peer_id, for UnregisterPeer and teardown on stream close.
        let mut registered: std::collections::HashMap<String, Uuid> = std::collections::HashMap::new();

        let result = loop {
            let msg = match stream.recv_frame().await {
                Ok(Some(MeshStreamFrame::Data { fenced: f, payload })) => {
                    // Re-fence every control frame, not just the Hello: a lease
                    // that moves mid-stream must reject subsequent registers.
                    if f != fenced {
                        break Err(MeshError::OwnerMismatch {
                            session_id,
                            generation: f.generation,
                            frame_owner_runtime_id: f.owner_runtime_id,
                            current_owner_runtime_id: self.local_runtime_id,
                        });
                    }
                    if let Err(e) = self.directory.validate(community_id, &f).await {
                        break Err(e);
                    }
                    match decode_control(&payload) {
                        Ok(m) => m,
                        Err(e) => break Err(e),
                    }
                }
                Ok(Some(MeshStreamFrame::Goodbye { .. })) | Ok(None) => break Ok(()),
                Ok(Some(other)) => {
                    break Err(MeshError::Transport(format!(
                        "huddle-control stream got unexpected frame {other:?}"
                    )));
                }
                Err(e) => break Err(e),
            };

            match msg {
                HuddleControlMsg::RegisterPeer {
                    pubkey,
                    protocol_version,
                } => {
                    let reply = self.register_remote_peer(
                        session_id,
                        fenced,
                        from,
                        &pubkey,
                        protocol_version,
                        &mut registered,
                    );
                    if let Err(e) = stream
                        .send_frame(MeshStreamFrame::Data {
                            fenced,
                            payload: encode_control(&reply)?,
                        })
                        .await
                    {
                        break Err(e);
                    }
                }
                HuddleControlMsg::UnregisterPeer { pubkey } => {
                    if let Some(peer_id) = registered.remove(&pubkey) {
                        if let Some(room) = self.rooms.get(session_id) {
                            room.remove_peer(peer_id);
                        }
                    }
                }
                // Owner→non-owner replies never arrive on the owner's accept
                // side; a peer sending one is a protocol violation.
                HuddleControlMsg::PeerRegistered { .. }
                | HuddleControlMsg::RegisterRejected { .. } => {
                    break Err(MeshError::Transport(
                        "huddle-control owner received an owner→non-owner reply".into(),
                    ));
                }
            }
        };

        // Teardown: drop every peer this stream registered, regardless of how
        // the loop ended. Dropping the peer drops its `audio_tx`, which ends the
        // matching `spawn_remote_peer_sink` task.
        if let Some(room) = self.rooms.get(session_id) {
            for (_pubkey, peer_id) in registered {
                room.remove_peer(peer_id);
            }
        }
        result
    }

    /// Admit one remote client into the owner's room and wire its fan-out back
    /// to the registering pod as datagrams. Returns the reply to send.
    fn register_remote_peer(
        &self,
        session_id: Uuid,
        fenced: FencedHeader,
        from: RuntimeId,
        pubkey: &str,
        protocol_version: u8,
        registered: &mut std::collections::HashMap<String, Uuid>,
    ) -> HuddleControlMsg {
        let room = self.rooms.get_or_create(session_id);
        match room.add_peer(pubkey.to_string(), protocol_version) {
            Ok((peer_id, peer_index, audio_rx, _peer_ctrl_rx)) => {
                registered.insert(pubkey.to_string(), peer_id);
                // The owner's Room fans out to this remote peer's `audio_tx`;
                // the sink drains `audio_rx` and ships each frame as a datagram
                // to the pod that hosts the client.
                spawn_remote_peer_sink(Arc::clone(&self.transport), from, fenced, audio_rx);
                HuddleControlMsg::PeerRegistered {
                    pubkey: pubkey.to_string(),
                    peer_index,
                }
            }
            Err(reason) => HuddleControlMsg::RegisterRejected {
                pubkey: pubkey.to_string(),
                reason: admission_to_rejection(reason),
            },
        }
    }
}

/// Map a room admission failure to the wire rejection taxonomy. Kept 1:1 with
/// the single-pod WS error codes so a cross-pod join surfaces the same
/// client-facing error a same-pod join would.
fn admission_to_rejection(err: AdmissionError) -> RegisterRejection {
    match err {
        AdmissionError::Full => RegisterRejection::RoomFull,
        AdmissionError::Ended => RegisterRejection::RoomEnded,
        AdmissionError::VersionMismatch { pinned, requested } => {
            RegisterRejection::VersionMismatch { pinned, requested }
        }
    }
}

/// The `Goodbye` reason a non-owner sends when its client leaves the huddle
/// cleanly. Re-exported so the handler's dial path uses one spelling.
pub const HUDDLE_SESSION_ENDED: GoodbyeReason = GoodbyeReason::SessionEnded;

// ── Non-owner-side HuddleControl dial path ───────────────────────────────────
//
// A client connected here whose huddle is owned by another pod. We keep the
// client as an ordinary local WS peer (heartbeats, `joined`/`left`, delivery of
// the owner's fan-out) but there is NO local fan-out: the owner is the sole
// fan-out authority. Each client Opus frame is shipped to the owner as a
// datagram tagged with the OWNER-assigned peer index; the owner fans out to
// everyone (including this pod's co-located clients, which hear each other via
// the owner round-trip — `deliver_prefixed` skips a client's own index so it
// never hears itself).

/// A registered cross-pod huddle session on the non-owner side.
///
/// Holds everything needed to forward the local client's media to the owner and
/// to unregister cleanly on disconnect. Media delivery *back* to the client goes
/// through the ordinary local room via `MeshAudioRouter::on_media_datagram`, so
/// this handle owns only the outbound (client→owner) half plus teardown.
pub struct RemoteHuddleSession {
    /// The owner-allocated peer index this client occupies in the owner's room.
    /// Stamped on every media datagram so the owner attributes frames correctly.
    peer_index: u8,
    /// Fenced header for this session's owner epoch; every datagram carries it.
    fenced: FencedHeader,
    /// The pod that owns the huddle.
    owner: RuntimeId,
    /// Pubkey of the local client, for the closing `UnregisterPeer`.
    pubkey: String,
    /// Transport for datagrams and the control-stream teardown.
    transport: Arc<dyn RelayPeerTransport>,
    /// The open `HuddleControl` stream to the owner; a clean `Goodbye` is sent
    /// on teardown.
    stream: MeshStream,
    /// Per-datagram monotonic sequence for loss/reorder observability.
    seq: u64,
}

/// Why a cross-pod join could not complete on the non-owner side.
#[derive(Debug)]
pub enum DialError {
    /// The owner refused the registration; surfaced to the client as the same
    /// WS error a same-pod join would produce, never a silent media drop.
    Rejected(RegisterRejection),
    /// Transport / protocol failure opening or serving the control stream.
    Mesh(MeshError),
}

impl From<MeshError> for DialError {
    fn from(e: MeshError) -> Self {
        DialError::Mesh(e)
    }
}

/// Open a `HuddleControl` stream to the owner and register the local client.
///
/// On success the owner has admitted the client as a remote peer and returned
/// its owner-assigned index; the returned [`RemoteHuddleSession`] forwards media
/// and unregisters on drop. On [`DialError::Rejected`] the caller surfaces the
/// owner's admission failure to the client unchanged.
pub async fn dial_remote_owner(
    transport: Arc<dyn RelayPeerTransport>,
    local_runtime_id: RuntimeId,
    owner: RuntimeId,
    fenced: FencedHeader,
    pubkey: String,
    protocol_version: u8,
) -> Result<RemoteHuddleSession, DialError> {
    let hello = StreamHello {
        sender: local_runtime_id,
        role: StreamRole::Session {
            fenced,
            profile: Profile::HuddleControl,
        },
    };
    // `open_session_stream` sends the Hello before returning.
    let mut stream = transport.open_session_stream(owner, hello).await?;

    stream
        .send_frame(MeshStreamFrame::Data {
            fenced,
            payload: encode_control(&HuddleControlMsg::RegisterPeer {
                pubkey: pubkey.clone(),
                protocol_version,
            })?,
        })
        .await?;

    match stream.recv_frame().await? {
        Some(MeshStreamFrame::Data { payload, .. }) => match decode_control(&payload)? {
            HuddleControlMsg::PeerRegistered { peer_index, .. } => Ok(RemoteHuddleSession {
                peer_index,
                fenced,
                owner,
                pubkey,
                transport,
                stream,
                seq: 0,
            }),
            HuddleControlMsg::RegisterRejected { reason, .. } => Err(DialError::Rejected(reason)),
            other => Err(DialError::Mesh(MeshError::Transport(format!(
                "expected PeerRegistered/RegisterRejected, got {other:?}"
            )))),
        },
        Some(MeshStreamFrame::Goodbye { .. }) | None => Err(DialError::Mesh(MeshError::Transport(
            "owner closed HuddleControl stream before replying".into(),
        ))),
        Some(other) => Err(DialError::Mesh(MeshError::Transport(format!(
            "unexpected HuddleControl frame from owner: {other:?}"
        )))),
    }
}

/// The `StreamHello.sender` for a dialed session: the fenced header carries the
/// owner's identity, but the *sender* is this pod. The owner validates
/// `hello.sender == authenticated peer`, so it must be our own runtime id — the
/// handler threads `local_runtime_id` in explicitly.
impl RemoteHuddleSession {
    /// The owner-assigned index this client occupies in the owner's room.
    pub fn peer_index(&self) -> u8 {
        self.peer_index
    }

    /// Forward one client Opus frame to the owner as a media datagram, tagged
    /// with the owner-assigned index. Drop-on-error: realtime audio never blocks
    /// on a slow or gone link (the same discipline as local fan-out).
    pub fn forward_media(&mut self, client_frame: &[u8]) {
        let dgram = media_datagram(self.peer_index, self.fenced, self.seq, client_frame);
        self.seq = self.seq.wrapping_add(1);
        if let Err(e) = self.transport.send_datagram(self.owner, dgram) {
            debug!(owner = %self.owner, "huddle media datagram to owner failed: {e}");
        }
    }

    /// Unregister the client from the owner and close the control stream
    /// cleanly. Best-effort: teardown never blocks connection cleanup.
    pub async fn close(mut self) {
        let _ = self
            .stream
            .send_frame(MeshStreamFrame::Data {
                fenced: self.fenced,
                payload: match encode_control(&HuddleControlMsg::UnregisterPeer {
                    pubkey: self.pubkey.clone(),
                }) {
                    Ok(p) => p,
                    Err(_) => return,
                },
            })
            .await;
        let _ = self
            .stream
            .send_frame(MeshStreamFrame::Goodbye {
                fenced: self.fenced,
                reason: HUDDLE_SESSION_ENDED,
            })
            .await;
    }
}

/// Build the media datagram a non-owner ships to the owner for one client
/// frame: `[owner_peer_index][client frame]`, stamped with the session fence
/// and sequence. Pure so the framing is unit-testable without a live transport
/// or stream.
fn media_datagram(
    peer_index: u8,
    fenced: FencedHeader,
    seq: u64,
    client_frame: &[u8],
) -> MeshDatagram {
    let mut payload = Vec::with_capacity(1 + client_frame.len());
    payload.push(peer_index);
    payload.extend_from_slice(client_frame);
    MeshDatagram {
        fenced,
        seq,
        payload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn rt(b: u8) -> RuntimeId {
        RuntimeId([b; 32])
    }

    fn community() -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(0xC0FFEE))
    }

    /// Scripted directory: `owner_of` returns a queued lookup, `acquire`
    /// returns a queued outcome, `validate` returns a queued result. Records
    /// call counts so ordering can be asserted.
    #[derive(Default)]
    struct FakeDir {
        owner: Mutex<Option<Ownership>>,
        acquire: Mutex<Option<AcquireOutcome>>,
        validate_fails: Mutex<bool>,
        acquire_calls: Mutex<u32>,
        validate_calls: Mutex<u32>,
    }

    impl FakeDir {
        fn owned_by(o: Ownership) -> Self {
            let d = Self::default();
            *d.owner.lock().unwrap() = Some(o);
            d
        }
        fn unowned_then_acquire(a: AcquireOutcome) -> Self {
            let d = Self::default();
            *d.acquire.lock().unwrap() = Some(a);
            d
        }
    }

    #[async_trait::async_trait]
    impl HuddleDirectory for FakeDir {
        async fn owner_of(
            &self,
            _c: CommunityId,
            _s: Uuid,
        ) -> Result<Option<Ownership>, MeshError> {
            Ok(*self.owner.lock().unwrap())
        }
        async fn acquire(
            &self,
            _c: CommunityId,
            _s: Uuid,
            _owner: RuntimeId,
        ) -> Result<AcquireOutcome, MeshError> {
            *self.acquire_calls.lock().unwrap() += 1;
            Ok(self.acquire.lock().unwrap().expect("acquire not scripted"))
        }
        async fn validate(&self, _c: CommunityId, _f: &FencedHeader) -> Result<(), MeshError> {
            *self.validate_calls.lock().unwrap() += 1;
            if *self.validate_fails.lock().unwrap() {
                Err(MeshError::OwnerMismatch {
                    session_id: Uuid::nil(),
                    generation: 0,
                    frame_owner_runtime_id: rt(0),
                    current_owner_runtime_id: rt(1),
                })
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn unowned_huddle_is_acquired_as_local_owner() {
        let dir = FakeDir::unowned_then_acquire(AcquireOutcome::Acquired(Ownership {
            owner_runtime_id: rt(1),
            generation: 7,
        }));
        let out = resolve_join(&dir, community(), Uuid::new_v4(), rt(1))
            .await
            .unwrap();
        assert_eq!(out, JoinOutcome::LocalOwner { generation: 7 });
        assert_eq!(*dir.acquire_calls.lock().unwrap(), 1);
        // No fence validation on the local-owner path — we ARE the lease.
        assert_eq!(*dir.validate_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn huddle_owned_by_us_is_local_owner_without_acquire() {
        let dir = FakeDir::owned_by(Ownership {
            owner_runtime_id: rt(1),
            generation: 3,
        });
        let out = resolve_join(&dir, community(), Uuid::new_v4(), rt(1))
            .await
            .unwrap();
        assert_eq!(out, JoinOutcome::LocalOwner { generation: 3 });
        // Live lease found → no acquire attempt.
        assert_eq!(*dir.acquire_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn huddle_owned_by_peer_is_remote_owner_and_fence_validated() {
        let dir = FakeDir::owned_by(Ownership {
            owner_runtime_id: rt(2),
            generation: 9,
        });
        let out = resolve_join(&dir, community(), Uuid::new_v4(), rt(1))
            .await
            .unwrap();
        assert_eq!(
            out,
            JoinOutcome::RemoteOwner {
                owner_runtime_id: rt(2),
                generation: 9,
            }
        );
        // The remote-owner path validates the fence before routing.
        assert_eq!(*dir.validate_calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn lost_acquire_race_routes_to_winner_as_remote_owner() {
        let dir = FakeDir::unowned_then_acquire(AcquireOutcome::Held(Ownership {
            owner_runtime_id: rt(2),
            generation: 4,
        }));
        let out = resolve_join(&dir, community(), Uuid::new_v4(), rt(1))
            .await
            .unwrap();
        assert_eq!(
            out,
            JoinOutcome::RemoteOwner {
                owner_runtime_id: rt(2),
                generation: 4,
            }
        );
        assert_eq!(*dir.acquire_calls.lock().unwrap(), 1);
        assert_eq!(*dir.validate_calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn remote_owner_fence_rejection_propagates() {
        let dir = FakeDir::owned_by(Ownership {
            owner_runtime_id: rt(2),
            generation: 9,
        });
        *dir.validate_fails.lock().unwrap() = true;
        let err = resolve_join(&dir, community(), Uuid::new_v4(), rt(1))
            .await
            .unwrap_err();
        assert!(matches!(err, MeshError::OwnerMismatch { .. }));
    }

    #[test]
    fn control_msg_roundtrips() {
        for msg in [
            HuddleControlMsg::RegisterPeer {
                pubkey: "abc123".into(),
                protocol_version: 2,
            },
            HuddleControlMsg::PeerRegistered {
                pubkey: "abc123".into(),
                peer_index: 42,
            },
            HuddleControlMsg::RegisterRejected {
                pubkey: "abc123".into(),
                reason: RegisterRejection::VersionMismatch {
                    pinned: 2,
                    requested: 1,
                },
            },
            HuddleControlMsg::RegisterRejected {
                pubkey: "abc123".into(),
                reason: RegisterRejection::Fenced(FenceRejection::StaleGeneration),
            },
            HuddleControlMsg::UnregisterPeer {
                pubkey: "abc123".into(),
            },
        ] {
            let bytes = encode_control(&msg).unwrap();
            assert_eq!(decode_control(&bytes).unwrap(), msg);
        }
    }

    #[test]
    fn fence_rejection_classifies_only_fence_errors() {
        assert_eq!(
            FenceRejection::from_mesh_error(&MeshError::StaleGeneration {
                session_id: Uuid::nil(),
                frame_generation: 1,
                known_generation: 2,
            }),
            Some(FenceRejection::StaleGeneration)
        );
        assert_eq!(
            FenceRejection::from_mesh_error(&MeshError::Transport("x".into())),
            None
        );
    }

    #[test]
    fn fenced_header_uses_local_id_for_local_owner_and_owner_id_for_remote() {
        let s = Uuid::new_v4();
        let local = JoinOutcome::LocalOwner { generation: 5 };
        assert_eq!(
            local.fenced_header(s, rt(1)),
            FencedHeader {
                session_id: s,
                generation: 5,
                owner_runtime_id: rt(1),
            }
        );
        let remote = JoinOutcome::RemoteOwner {
            owner_runtime_id: rt(2),
            generation: 8,
        };
        assert_eq!(
            remote.fenced_header(s, rt(1)),
            FencedHeader {
                session_id: s,
                generation: 8,
                owner_runtime_id: rt(2),
            }
        );
    }

    #[test]
    fn admission_errors_map_to_wire_rejections() {
        assert_eq!(
            admission_to_rejection(AdmissionError::Full),
            RegisterRejection::RoomFull
        );
        assert_eq!(
            admission_to_rejection(AdmissionError::Ended),
            RegisterRejection::RoomEnded
        );
        assert_eq!(
            admission_to_rejection(AdmissionError::VersionMismatch {
                pinned: 2,
                requested: 1
            }),
            RegisterRejection::VersionMismatch {
                pinned: 2,
                requested: 1
            }
        );
    }

    #[test]
    fn media_datagram_tags_owner_index_and_stamps_fence() {
        let fenced = FencedHeader {
            session_id: Uuid::new_v4(),
            generation: 9,
            owner_runtime_id: rt(2),
        };
        // Owner-assigned index is the first payload byte; client bytes follow.
        let d0 = media_datagram(42, fenced, 0, &[0xDE, 0xAD]);
        assert_eq!(d0.payload, vec![42, 0xDE, 0xAD]);
        assert_eq!(d0.fenced, fenced);
        assert_eq!(d0.seq, 0);
        // Empty client frame still carries the index byte (owner tolerates it).
        let d1 = media_datagram(7, fenced, 3, &[]);
        assert_eq!(d1.payload, vec![7]);
        assert_eq!(d1.seq, 3);
    }
}
