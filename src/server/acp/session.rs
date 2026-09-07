//! What an ACP session *is*, on this host.
//!
//! ACP gives a session an opaque id, a `cwd` and a set of MCP servers. None of
//! those mean here what they mean for a local harness, and the translation is
//! where the interesting decisions are.
//!
//! A session is the triple **(company, thread, optional agent)**. The thread
//! the turns land in is the desk the client asked for, or — when the client
//! pins the session to a roster member — that member's DM channel
//! (`dm:<member>`, the one chat key the cycle's routing resolves to that
//! member). Either way an ACP client and the web console looking at the same
//! thread see the same conversation, which is the whole reason to reuse the
//! thread rather than invent a parallel one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::ports::types::CompanyId;
use crate::runtime::assignee::DM_PREFIX;

/// One live ACP session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpSession {
    pub id: String,
    pub company: CompanyId,
    /// The thread these turns land in — the same key the console shows. A
    /// pinned session's is the pinned member's DM channel; an unpinned one's is
    /// the desk the client asked for (see [`Self::thread_key`]).
    pub chat: String,
    /// Pins the session to one roster member. `None` routes normally, through
    /// the orchestrator and the desk lead.
    pub agent_id: Option<String>,
}

impl AcpSession {
    /// The chat key a session's turns land in, given the desk the client asked
    /// for.
    ///
    /// A pinned session is answered by its member, and `responder_for` resolves
    /// a chat key to a member only through the console's own DM channel shape
    /// (`dm:<member>`, spelled by [`crate::runtime::assignee::dm_key`]). The
    /// requested desk is therefore superseded by the member's DM channel; an
    /// unpinned session keeps the desk.
    pub fn thread_key(requested_chat: &str, agent_id: Option<&str>) -> String {
        match agent_id {
            Some(id) => format!("{DM_PREFIX}{id}"),
            None => requested_chat.to_string(),
        }
    }
}

/// Why a `session/new` was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewSessionRefusal {
    /// The client asked for MCP servers.
    McpServers,
    /// The client asked for extra directories.
    AdditionalDirectories,
}

impl NewSessionRefusal {
    /// What to tell the client. Specific, because a client that is told only
    /// "invalid params" will retry with the same request.
    pub fn message(&self) -> &'static str {
        match self {
            Self::McpServers => {
                "this host does not accept session-scoped MCP servers; configure them on the \
                 company (POST /api/v1/companies/{id}/mcp/servers) and they apply to every session"
            }
            Self::AdditionalDirectories => {
                "this host does not accept additional directories; an agent's workspace is \
                 server-side and fixed"
            }
        }
    }
}

/// Checks the parts of `session/new` this host cannot honour.
///
/// **Refused, not ignored.** Silently dropping `mcpServers` would leave a
/// client believing its tools were installed, and the model would then be asked
/// why it never called them. And they cannot be honoured: MCP servers here are
/// durable per-company configuration behind an admin gate, materialised into
/// the harness by a fingerprint rebuild. A session-scoped injection would
/// bypass the gate, force a pool rebuild per session, and leak across sessions
/// because the pool is per (company, agent).
pub fn refuse_unsupported(
    mcp_servers: &[serde_json::Value],
    additional_directories: &[serde_json::Value],
) -> Option<NewSessionRefusal> {
    if !mcp_servers.is_empty() {
        return Some(NewSessionRefusal::McpServers);
    }
    if !additional_directories.is_empty() {
        return Some(NewSessionRefusal::AdditionalDirectories);
    }
    None
}

/// What this host tells a client about the `cwd` it asked for.
///
/// ACP mandates an absolute path, and a client sends one that is meaningful on
/// **its** machine. On a remote host it names nothing. Rejecting would break
/// every stock ACP client, which always sends one; pretending to honour it
/// would break every file tool, which would resolve against a directory that
/// does not exist.
///
/// So it is accepted, ignored, and *reported* — the client is told the real
/// root in `_meta` and that its own was not used.
pub fn cwd_meta(server_workspace: &str) -> serde_json::Value {
    serde_json::json!({
        "opencompany/workspace": server_workspace,
        "opencompany/cwdIgnored": true,
    })
}

/// The most ACP sessions one connection id may hold open at once.
///
/// A `connectionId` is caller-supplied and otherwise unbounded, so a caller
/// minting one session after another on the same id would grow that
/// connection's entry forever.
pub const MAX_SESSIONS_PER_CONNECTION: usize = 32;

/// The most ACP sessions this host holds open at once, across every
/// connection. A circuit breaker on total memory, not a business limit.
pub const MAX_SESSIONS_TOTAL: usize = 8_192;

/// How long an ACP session may sit unused before [`SessionRegistry::sweep_expired`]
/// reclaims it. This surface has no heartbeat, so the bound has to be
/// generous enough to outlast a normal gap between prompts — one working day.
pub const SESSION_TTL_MILLIS: u64 = 24 * 60 * 60 * 1000;

/// How often [`SessionSweeper`] checks for expired sessions.
///
/// Deliberately much shorter than [`SESSION_TTL_MILLIS`]: a session that goes
/// idle right after one sweep tick is not stale enough for the *next* tick
/// (a full [`SESSION_TTL_MILLIS`] later) to reclaim either, so sweeping once
/// per TTL lets a session squat its cap slot for up to twice its own TTL. An
/// hourly cadence bounds that overshoot to about an hour instead.
pub const SESSION_SWEEP_INTERVAL_MILLIS: u64 = 60 * 60 * 1000;

const _: () = assert!(
    SESSION_SWEEP_INTERVAL_MILLIS < SESSION_TTL_MILLIS,
    "sweeping no more often than the TTL reintroduces the up-to-2x-TTL overshoot this constant exists to bound",
);

/// Why [`SessionRegistry::open`] refused a `session/new`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenSessionRefusal {
    /// The connection id is already bound to a different caller, or this
    /// caller never opened it.
    NotOwned,
    /// The connection already holds [`MAX_SESSIONS_PER_CONNECTION`] sessions.
    PerConnectionCap,
    /// The host already holds [`MAX_SESSIONS_TOTAL`] sessions.
    TotalCap,
}

impl OpenSessionRefusal {
    pub fn message(&self) -> &'static str {
        match self {
            // Deliberately the same wording as an unrecognized connection id:
            // telling the two apart would let a caller learn that a foreign
            // connection id exists by probing it.
            Self::NotOwned => "unknown ACP connection",
            Self::PerConnectionCap => "too many open ACP sessions on this connection",
            Self::TotalCap => "too many open ACP sessions on this host",
        }
    }
}

#[derive(Debug)]
struct SessionEntry {
    session: Arc<AcpSession>,
    last_used_millis: u64,
}

/// One connection's sessions, plus who is allowed to address it.
#[derive(Debug)]
struct Connection {
    owner: String,
    sessions: HashMap<String, SessionEntry>,
}

/// The live sessions on this host, keyed by connection so a disconnect can
/// sweep them.
///
/// A connection id is bound to whichever caller first opens a session on it
/// (see [`SessionRegistry::open`]); every other method refuses an id it does
/// not recognize as that same caller's, rather than acting on whatever the
/// request claims.
#[derive(Debug, Default)]
pub struct SessionRegistry {
    by_connection: Mutex<HashMap<String, Connection>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a session on `connection`, binding it to `owner` if this is the
    /// first session opened on that id. Refuses an id already bound to a
    /// different owner, and refuses once either cap is hit.
    pub fn open(
        &self,
        connection: &str,
        owner: &str,
        session: AcpSession,
        now_millis: u64,
    ) -> Result<Arc<AcpSession>, OpenSessionRefusal> {
        let mut by_connection = self
            .by_connection
            .lock()
            .expect("session registry poisoned");
        if let Some(existing) = by_connection.get(connection) {
            if existing.owner != owner {
                return Err(OpenSessionRefusal::NotOwned);
            }
            if existing.sessions.len() >= MAX_SESSIONS_PER_CONNECTION {
                return Err(OpenSessionRefusal::PerConnectionCap);
            }
        }
        let total: usize = by_connection.values().map(|c| c.sessions.len()).sum();
        if total >= MAX_SESSIONS_TOTAL {
            return Err(OpenSessionRefusal::TotalCap);
        }
        let session = Arc::new(session);
        let conn = by_connection
            .entry(connection.to_string())
            .or_insert_with(|| Connection {
                owner: owner.to_string(),
                sessions: HashMap::new(),
            });
        conn.sessions.insert(
            session.id.clone(),
            SessionEntry {
                session: Arc::clone(&session),
                last_used_millis: now_millis,
            },
        );
        Ok(session)
    }

    /// Looks up a session for `owner`, refusing a connection id it does not
    /// hold. Renews the session's idle TTL on a hit; evicts and refuses one
    /// already past [`SESSION_TTL_MILLIS`] instead — otherwise a lookup that
    /// lands in the gap between two [`SessionSweeper`] ticks would revive an
    /// already-expired session merely by touching it.
    pub fn get(
        &self,
        connection: &str,
        owner: &str,
        id: &str,
        now_millis: u64,
    ) -> Option<Arc<AcpSession>> {
        let mut by_connection = self
            .by_connection
            .lock()
            .expect("session registry poisoned");
        let conn = by_connection.get_mut(connection)?;
        if conn.owner != owner {
            return None;
        }
        if conn
            .sessions
            .get(id)
            .is_some_and(|e| Self::expired(e, now_millis))
        {
            conn.sessions.remove(id);
            if conn.sessions.is_empty() {
                by_connection.remove(connection);
            }
            return None;
        }
        let entry = conn.sessions.get_mut(id)?;
        entry.last_used_millis = now_millis;
        Some(Arc::clone(&entry.session))
    }

    /// Looks up a session for `owner` without renewing its idle TTL, refusing
    /// a connection id it does not hold. Evicts and refuses one already past
    /// [`SESSION_TTL_MILLIS`], for the same reason [`Self::get`] does — a
    /// caller cannot authorize itself to act on a session that has already
    /// aged out, no matter which lookup finds it.
    ///
    /// For a caller that still has to run `authorize_address` on the
    /// session's company before it may act on it — the ACP transport's
    /// `session/delete` and `session/prompt`. Renewing the TTL via [`Self::get`]
    /// before that check can still refuse the caller would let a tenant whose
    /// access to one company under it was revoked keep a session's cap slot
    /// alive indefinitely, by repeatedly presenting it and losing the
    /// authorization check every time.
    pub fn peek(
        &self,
        connection: &str,
        owner: &str,
        id: &str,
        now_millis: u64,
    ) -> Option<Arc<AcpSession>> {
        let mut by_connection = self
            .by_connection
            .lock()
            .expect("session registry poisoned");
        let conn = by_connection.get_mut(connection)?;
        if conn.owner != owner {
            return None;
        }
        if conn
            .sessions
            .get(id)
            .is_some_and(|e| Self::expired(e, now_millis))
        {
            conn.sessions.remove(id);
            if conn.sessions.is_empty() {
                by_connection.remove(connection);
            }
            return None;
        }
        Some(Arc::clone(&conn.sessions.get(id)?.session))
    }

    /// Whether `entry` has sat idle longer than [`SESSION_TTL_MILLIS`].
    fn expired(entry: &SessionEntry, now_millis: u64) -> bool {
        now_millis.saturating_sub(entry.last_used_millis) > SESSION_TTL_MILLIS
    }

    /// Renews a session's idle TTL, once the caller's authorization to act on
    /// it is already confirmed (typically via a prior [`Self::peek`]).
    ///
    /// A silent no-op for a connection or session `owner` does not hold, or
    /// that vanished between the authorization check and this call — nothing
    /// here needs a second refusal for a session that already will not run.
    pub fn touch(&self, connection: &str, owner: &str, id: &str, now_millis: u64) {
        let mut by_connection = self
            .by_connection
            .lock()
            .expect("session registry poisoned");
        if let Some(conn) = by_connection.get_mut(connection)
            && conn.owner == owner
            && let Some(entry) = conn.sessions.get_mut(id)
        {
            entry.last_used_millis = now_millis;
        }
    }

    /// Every session on a connection `owner` holds, for `session/list`.
    /// `None` when the connection is unknown or belongs to someone else.
    pub fn list(
        &self,
        connection: &str,
        owner: &str,
        now_millis: u64,
    ) -> Option<Vec<Arc<AcpSession>>> {
        let by_connection = self
            .by_connection
            .lock()
            .expect("session registry poisoned");
        let conn = by_connection.get(connection)?;
        if conn.owner != owner {
            return None;
        }
        Some(
            conn.sessions
                .values()
                .filter(|entry| !Self::expired(entry, now_millis))
                .map(|entry| Arc::clone(&entry.session))
                .collect(),
        )
    }

    /// Drops one session, for ACP's `session/delete`.
    ///
    /// Returns whether the session existed. Deleting a session that was never
    /// there — or addressing a connection `owner` does not hold — is a silent
    /// no-op, exactly as ACP specifies for an opaque id: absence says nothing
    /// useful.
    pub fn remove(&self, connection: &str, owner: &str, id: &str) -> bool {
        let mut by_connection = self
            .by_connection
            .lock()
            .expect("session registry poisoned");
        let Some(conn) = by_connection.get_mut(connection) else {
            return false;
        };
        if conn.owner != owner {
            return false;
        }
        let removed = conn.sessions.remove(id).is_some();
        // A connection's last session going also takes the connection key
        // (and its owner binding) with it — otherwise `session/new` +
        // `session/delete` over fresh caller-controlled connection ids grows
        // the host-wide map by one empty entry per connection, forever.
        if conn.sessions.is_empty() {
            by_connection.remove(connection);
        }
        removed
    }

    /// Drops every session a connection `owner` holds that `authorized`
    /// approves, under one lock, and reports the ids actually removed. A
    /// connection `owner` does not hold is left untouched — reports none.
    ///
    /// `authorized` is a per-session check, not a blanket one, because
    /// `owner` names a tenant rather than an authorization scope: two
    /// platform credentials for the same tenant can carry different company
    /// allow-lists (`PlatformClaims::companies`), and a connection can hold
    /// sessions opened under either. Removing every session on the
    /// connection unconditionally would let a narrowly-scoped credential
    /// close sessions for companies outside its own allow-list, just because
    /// it shares an owner string with whichever credential opened them.
    /// Checked under the same lock as the removal so a concurrent
    /// `session/new` cannot land in a gap between the check and the removal.
    pub fn close_connection(
        &self,
        connection: &str,
        owner: &str,
        mut authorized: impl FnMut(&CompanyId) -> bool,
    ) -> Vec<String> {
        let mut by_connection = self
            .by_connection
            .lock()
            .expect("session registry poisoned");
        let Some(conn) = by_connection.get_mut(connection) else {
            return Vec::new();
        };
        if conn.owner != owner {
            return Vec::new();
        }
        let mut removed = Vec::new();
        conn.sessions.retain(|id, entry| {
            if authorized(&entry.session.company) {
                removed.push(id.clone());
                false
            } else {
                true
            }
        });
        if conn.sessions.is_empty() {
            by_connection.remove(connection);
        }
        removed
    }

    /// Forgets every session idle past [`SESSION_TTL_MILLIS`], and every
    /// connection that leaves empty. Returns how many sessions were reclaimed.
    pub fn sweep_expired(&self, now_millis: u64) -> usize {
        let mut by_connection = self
            .by_connection
            .lock()
            .expect("session registry poisoned");
        let mut removed = 0;
        by_connection.retain(|_, conn| {
            let before = conn.sessions.len();
            conn.sessions.retain(|_, entry| {
                now_millis.saturating_sub(entry.last_used_millis) <= SESSION_TTL_MILLIS
            });
            removed += before - conn.sessions.len();
            !conn.sessions.is_empty()
        });
        removed
    }
}

/// Periodically reclaims ACP sessions idle past [`SESSION_TTL_MILLIS`].
///
/// Mirrors [`crate::server::presence::PresenceSweeper`]: this registry is
/// host-global, not scoped to a registered company, so it gets its own
/// always-on task rather than riding the per-company maintenance ticker.
pub struct SessionSweeper {
    registry: Arc<SessionRegistry>,
}

impl SessionSweeper {
    pub fn new(registry: Arc<SessionRegistry>) -> Self {
        Self { registry }
    }

    /// Runs until `shutdown` is notified, sweeping every
    /// [`SESSION_SWEEP_INTERVAL_MILLIS`].
    pub fn spawn(self, shutdown: Arc<Notify>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let notified = shutdown.notified();
            tokio::pin!(notified);
            loop {
                tokio::select! {
                    _ = &mut notified => break,
                    _ = tokio::time::sleep(Duration::from_millis(SESSION_SWEEP_INTERVAL_MILLIS)) => {
                        self.registry.sweep_expired(crate::ports::now_millis());
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;

    fn session(id: &str, company: &str) -> AcpSession {
        AcpSession {
            id: id.to_string(),
            company: CompanyId::new(company),
            chat: "General".to_string(),
            agent_id: None,
        }
    }

    #[test]
    fn session_scoped_mcp_servers_are_refused_with_a_reason() {
        // Silently dropping them leaves a client believing its tools were
        // installed, and the model then gets asked why it never called them.
        let refusal = refuse_unsupported(&[json!({ "name": "x" })], &[]).unwrap();
        assert_eq!(refusal, NewSessionRefusal::McpServers);
        // Specific enough to act on: a client told only "invalid params"
        // retries with the same request.
        assert!(refusal.message().contains("mcp/servers"));
    }

    #[test]
    fn additional_directories_are_refused_too() {
        let refusal = refuse_unsupported(&[], &[json!("/tmp")]).unwrap();
        assert_eq!(refusal, NewSessionRefusal::AdditionalDirectories);
    }

    #[test]
    fn an_ordinary_request_is_accepted() {
        assert!(refuse_unsupported(&[], &[]).is_none());
    }

    #[test]
    fn the_client_is_told_its_cwd_was_not_used() {
        // Accepted and ignored is only honest if it is also reported. A client
        // that believes its own path was honoured will resolve file paths
        // against a directory that does not exist on this machine.
        let meta = cwd_meta("/data/harness/acme/ceo/workspace");
        assert_eq!(meta["opencompany/cwdIgnored"], true);
        assert_eq!(
            meta["opencompany/workspace"],
            "/data/harness/acme/ceo/workspace"
        );
    }

    #[test]
    fn sessions_are_scoped_to_their_connection() {
        // Two clients must not see each other's sessions — and a reconnecting
        // one must not resume into another's.
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        registry
            .open("conn-b", "bob", session("s2", "globex"), 0)
            .unwrap();

        assert!(registry.get("conn-a", "alice", "s1", 0).is_some());
        assert!(
            registry.get("conn-b", "alice", "s1", 0).is_none(),
            "no cross-connection reads"
        );
        assert_eq!(registry.list("conn-a", "alice", 0).unwrap().len(), 1);
    }

    #[test]
    fn a_caller_cannot_open_a_session_on_a_connection_it_does_not_own() {
        // The defect this registry exists to close: a caller-supplied
        // `connectionId` is otherwise just a guessable string, and whoever
        // guesses it could open sessions into somebody else's connection.
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();

        let refusal = registry
            .open("conn-a", "mallory", session("s2", "acme"), 0)
            .unwrap_err();
        assert_eq!(refusal, OpenSessionRefusal::NotOwned);
        assert_eq!(
            registry.list("conn-a", "alice", 0).unwrap().len(),
            1,
            "the attempted takeover left alice's session set untouched"
        );
        assert!(
            registry.list("conn-a", "mallory", 0).is_none(),
            "mallory never owned the connection, so it stays invisible to her too"
        );
    }

    #[test]
    fn a_caller_cannot_read_or_act_on_a_connection_it_does_not_own() {
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();

        assert!(registry.get("conn-a", "mallory", "s1", 0).is_none());
        assert!(registry.list("conn-a", "mallory", 0).is_none());
        assert!(!registry.remove("conn-a", "mallory", "s1"));
        assert!(
            registry
                .close_connection("conn-a", "mallory", |_| true)
                .is_empty()
        );
        // None of mallory's attempts touched alice's session.
        assert!(registry.get("conn-a", "alice", "s1", 0).is_some());
    }

    #[test]
    fn a_session_over_the_per_connection_cap_is_refused() {
        let registry = SessionRegistry::new();
        for i in 0..MAX_SESSIONS_PER_CONNECTION {
            registry
                .open("conn-a", "alice", session(&format!("s{i}"), "acme"), 0)
                .unwrap();
        }
        let refusal = registry
            .open("conn-a", "alice", session("s-over", "acme"), 0)
            .unwrap_err();
        assert_eq!(refusal, OpenSessionRefusal::PerConnectionCap);
        assert_eq!(
            registry.list("conn-a", "alice", 0).unwrap().len(),
            MAX_SESSIONS_PER_CONNECTION
        );
    }

    #[test]
    fn a_session_over_the_host_wide_cap_is_refused_even_on_a_fresh_connection() {
        let registry = SessionRegistry::new();
        // Fan the total cap out across many connections rather than one, so
        // this proves the cap is host-wide and not just per-connection.
        let mut opened = 0;
        for i in 0..MAX_SESSIONS_TOTAL {
            let conn = format!("conn-{i}");
            registry
                .open(&conn, "alice", session("s0", "acme"), 0)
                .unwrap();
            opened += 1;
        }
        assert_eq!(opened, MAX_SESSIONS_TOTAL);
        let refusal = registry
            .open("conn-fresh", "alice", session("s0", "acme"), 0)
            .unwrap_err();
        assert_eq!(refusal, OpenSessionRefusal::TotalCap);
    }

    #[test]
    fn an_idle_session_past_its_ttl_is_swept() {
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();

        assert_eq!(
            registry.sweep_expired(SESSION_TTL_MILLIS),
            0,
            "exactly at the boundary is not yet expired"
        );
        assert_eq!(registry.sweep_expired(SESSION_TTL_MILLIS + 1), 1);
        assert!(
            registry
                .get("conn-a", "alice", "s1", SESSION_TTL_MILLIS + 1)
                .is_none()
        );
    }

    #[test]
    fn using_a_session_renews_its_idle_ttl() {
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        // A `get` at half the TTL — a live prompt — renews the clock.
        assert!(
            registry
                .get("conn-a", "alice", "s1", SESSION_TTL_MILLIS / 2)
                .is_some()
        );
        assert_eq!(
            registry.sweep_expired(SESSION_TTL_MILLIS),
            0,
            "renewed at TTL/2, so a full TTL later it is not yet idle that long"
        );
        assert!(
            registry
                .get("conn-a", "alice", "s1", SESSION_TTL_MILLIS)
                .is_some()
        );
    }

    #[test]
    fn peek_does_not_renew_the_idle_ttl() {
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        // A `peek` at half the TTL — a pre-authorization check — must not
        // extend the clock the way `get` does.
        assert!(
            registry
                .peek("conn-a", "alice", "s1", SESSION_TTL_MILLIS / 2)
                .is_some()
        );
        assert_eq!(
            registry.sweep_expired(SESSION_TTL_MILLIS + 1),
            1,
            "peek must not have renewed the session past its original TTL"
        );
    }

    #[test]
    fn peek_refuses_a_connection_or_owner_it_does_not_hold() {
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        assert!(registry.peek("conn-b", "alice", "s1", 0).is_none());
        assert!(registry.peek("conn-a", "mallory", "s1", 0).is_none());
        assert!(registry.peek("conn-a", "alice", "s1", 0).is_some());
    }

    #[test]
    fn touch_renews_the_idle_ttl_that_peek_left_alone() {
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        assert!(registry.peek("conn-a", "alice", "s1", 0).is_some());
        registry.touch("conn-a", "alice", "s1", SESSION_TTL_MILLIS / 2);
        assert_eq!(
            registry.sweep_expired(SESSION_TTL_MILLIS),
            0,
            "touch renewed at TTL/2, so a full TTL later it is not yet idle that long"
        );
    }

    #[test]
    fn peek_cannot_revive_a_session_already_past_its_ttl() {
        // Landing in the gap between two sweeper ticks must not let a lookup
        // (and the touch a caller runs after it authorizes) revive a session
        // that is already stale — the sweep is a cleanup convenience, not the
        // only place staleness is enforced (codex review).
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        assert!(
            registry
                .peek("conn-a", "alice", "s1", SESSION_TTL_MILLIS + 1)
                .is_none(),
            "a peek past the TTL must refuse, not hand back a session to authorize and touch"
        );
        // And it evicted the stale entry rather than merely refusing this call.
        assert!(registry.list("conn-a", "alice", 0).is_none());
    }

    #[test]
    fn get_cannot_revive_a_session_already_past_its_ttl() {
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        assert!(
            registry
                .get("conn-a", "alice", "s1", SESSION_TTL_MILLIS + 1)
                .is_none()
        );
    }

    #[test]
    fn list_does_not_advertise_a_session_already_past_its_ttl() {
        // `session/list` must not tell a caller a session is there when
        // `peek` and `get` already refuse it as expired, landing in the same
        // gap between two hourly sweeper ticks — otherwise a client is told a
        // session is resumable and then has `session/prompt` immediately
        // report it unknown (codex review).
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        registry
            .open("conn-a", "alice", session("s2", "acme"), SESSION_TTL_MILLIS)
            .unwrap();
        let listed = registry
            .list("conn-a", "alice", SESSION_TTL_MILLIS + 1)
            .expect("the connection itself is not expired, only one session on it");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "s2");
    }

    #[test]
    fn touch_is_a_silent_no_op_for_a_connection_or_owner_it_does_not_hold() {
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        // Neither call may panic, and neither may renew alice's session.
        registry.touch("conn-b", "alice", "s1", SESSION_TTL_MILLIS / 2);
        registry.touch("conn-a", "mallory", "s1", SESSION_TTL_MILLIS / 2);
        assert_eq!(registry.sweep_expired(SESSION_TTL_MILLIS + 1), 1);
    }

    #[test]
    fn sweeping_prunes_the_connection_once_every_session_on_it_expires() {
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        registry.sweep_expired(SESSION_TTL_MILLIS + 1);
        let by_connection = registry
            .by_connection
            .lock()
            .expect("session registry poisoned");
        assert!(!by_connection.contains_key("conn-a"));
    }

    #[test]
    fn closing_a_connection_drops_its_sessions_and_nothing_else() {
        // Without this a client that reconnects repeatedly accumulates sessions
        // nothing will ever close.
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        registry
            .open("conn-b", "bob", session("s2", "acme"), 0)
            .unwrap();

        let closed = registry.close_connection("conn-a", "alice", |_| true);
        assert_eq!(closed, vec!["s1".to_string()]);
        assert!(registry.get("conn-a", "alice", "s1", 0).is_none());
        assert!(
            registry.get("conn-b", "bob", "s2", 0).is_some(),
            "other connections survive"
        );
    }

    #[test]
    fn close_connection_leaves_a_session_the_caller_is_not_authorized_for() {
        // `owner` names a tenant, not an authorization scope: two platform
        // credentials for the same tenant can carry different company
        // allow-lists, so a connection can hold sessions the *presented*
        // credential is not itself authorized to act on (coderabbit review).
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "platform:acme", session("s-allowed", "acme"), 0)
            .unwrap();
        registry
            .open(
                "conn-a",
                "platform:acme",
                session("s-restricted", "globex"),
                0,
            )
            .unwrap();

        let closed = registry.close_connection("conn-a", "platform:acme", |company| {
            company.as_ref() == "acme"
        });
        assert_eq!(closed, vec!["s-allowed".to_string()]);
        assert!(
            registry
                .get("conn-a", "platform:acme", "s-restricted", 0)
                .is_some(),
            "a session for a company outside the caller's own allow-list must survive its disconnect"
        );
    }

    #[test]
    fn disconnect_sweeps_every_session_it_opened_with_none_stranded() {
        let registry = SessionRegistry::new();
        for i in 0..5 {
            registry
                .open("conn-a", "alice", session(&format!("s{i}"), "acme"), 0)
                .unwrap();
        }
        let closed = registry.close_connection("conn-a", "alice", |_| true);
        assert_eq!(closed.len(), 5);
        for i in 0..5 {
            assert!(
                registry
                    .get("conn-a", "alice", &format!("s{i}"), 0)
                    .is_none(),
                "session s{i} was stranded"
            );
        }
        assert!(registry.list("conn-a", "alice", 0).is_none());
    }

    #[test]
    fn deleting_a_session_removes_only_that_session() {
        // ACP's `session/delete`: one session goes, the connection and its
        // other sessions survive.
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        registry
            .open("conn-a", "alice", session("s2", "acme"), 0)
            .unwrap();

        assert!(registry.remove("conn-a", "alice", "s1"));
        assert!(registry.get("conn-a", "alice", "s1", 0).is_none());
        assert!(registry.get("conn-a", "alice", "s2", 0).is_some());
        assert_eq!(registry.list("conn-a", "alice", 0).unwrap().len(), 1);
    }

    #[test]
    fn removing_a_connections_last_session_prunes_the_connection() {
        // `session/new` + `session/disconnect` over fresh caller-controlled
        // connection ids must not grow the host-wide registry by one empty map
        // per connection, forever.
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        registry
            .open("conn-b", "bob", session("s2", "acme"), 0)
            .unwrap();

        assert!(registry.remove("conn-a", "alice", "s1"));
        let by_connection = registry
            .by_connection
            .lock()
            .expect("session registry poisoned");
        assert!(
            !by_connection.contains_key("conn-a"),
            "the emptied connection key is pruned, not left as an empty map"
        );
        assert!(
            by_connection.contains_key("conn-b"),
            "a connection that still holds sessions survives"
        );
    }

    #[test]
    fn deleting_a_never_existing_session_is_a_silent_no_op() {
        // ACP says deleting an already-deleted or never-existing session should
        // succeed silently — an opaque id leaking "I never had that" by an
        // error would tell a caller more than it needs to know.
        let registry = SessionRegistry::new();
        registry
            .open("conn-a", "alice", session("s1", "acme"), 0)
            .unwrap();
        assert!(!registry.remove("conn-a", "alice", "ghost"));
        assert!(!registry.remove("conn-b", "alice", "s1"));
        assert_eq!(registry.list("conn-a", "alice", 0).unwrap().len(), 1);
    }

    #[test]
    fn a_session_names_its_company_and_desk() {
        // The triple is what makes an ACP session and the console's view of the
        // same desk one conversation rather than two.
        let s = session("s1", "acme");
        assert_eq!(s.company, CompanyId::new("acme"));
        assert_eq!(s.chat, "General");
        assert!(
            s.agent_id.is_none(),
            "unpinned routes through the desk lead"
        );
    }

    #[test]
    fn a_pinned_sessions_thread_is_the_members_dm_channel() {
        // A pin is answered by its member, and `responder_for` resolves a chat
        // key to a member only through the `dm:<member>` shape — so that is the
        // thread key, not the desk the client asked for.
        let pin = Some("ceo".to_string());
        assert_eq!(AcpSession::thread_key("General", pin.as_deref()), "dm:ceo");
        assert_eq!(AcpSession::thread_key("General", None), "General");
    }
}
