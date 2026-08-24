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

/// The live sessions on this host, keyed by connection so a disconnect can
/// sweep them.
#[derive(Debug, Default)]
pub struct SessionRegistry {
    by_connection: Mutex<HashMap<String, HashMap<String, Arc<AcpSession>>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, connection: &str, session: AcpSession) -> Arc<AcpSession> {
        let session = Arc::new(session);
        self.by_connection
            .lock()
            .expect("session registry poisoned")
            .entry(connection.to_string())
            .or_default()
            .insert(session.id.clone(), Arc::clone(&session));
        session
    }

    pub fn get(&self, connection: &str, id: &str) -> Option<Arc<AcpSession>> {
        self.by_connection
            .lock()
            .expect("session registry poisoned")
            .get(connection)
            .and_then(|sessions| sessions.get(id).cloned())
    }

    /// Every session on a connection, for `session/list`.
    pub fn list(&self, connection: &str) -> Vec<Arc<AcpSession>> {
        self.by_connection
            .lock()
            .expect("session registry poisoned")
            .get(connection)
            .map(|sessions| sessions.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Drops one session, for ACP's `session/delete`.
    ///
    /// Returns whether the session existed. Deleting a session that was never
    /// there is a silent no-op, exactly as ACP specifies — an opaque id says
    /// nothing useful by its absence.
    pub fn remove(&self, connection: &str, id: &str) -> bool {
        let mut by_connection = self
            .by_connection
            .lock()
            .expect("session registry poisoned");
        let removed = by_connection
            .get_mut(connection)
            .map(|sessions| sessions.remove(id).is_some())
            .unwrap_or(false);
        // A connection's last session going also takes the connection key with
        // it. Without this, `session/new` + `session/delete` (or `disconnect`)
        // over fresh caller-controlled connection ids grows the host-wide map
        // by one empty entry per connection, forever.
        if by_connection
            .get(connection)
            .is_some_and(|sessions| sessions.is_empty())
        {
            by_connection.remove(connection);
        }
        removed
    }

    /// Drops every session a connection held.
    ///
    /// Keyed by connection precisely so this is possible: without it, a client
    /// that reconnects repeatedly accumulates sessions nothing will ever close.
    pub fn close_connection(&self, connection: &str) {
        self.by_connection
            .lock()
            .expect("session registry poisoned")
            .remove(connection);
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
        registry.insert("conn-a", session("s1", "acme"));
        registry.insert("conn-b", session("s2", "globex"));

        assert!(registry.get("conn-a", "s1").is_some());
        assert!(
            registry.get("conn-b", "s1").is_none(),
            "no cross-connection reads"
        );
        assert_eq!(registry.list("conn-a").len(), 1);
    }

    #[test]
    fn closing_a_connection_drops_its_sessions_and_nothing_else() {
        // Without this a client that reconnects repeatedly accumulates sessions
        // nothing will ever close.
        let registry = SessionRegistry::new();
        registry.insert("conn-a", session("s1", "acme"));
        registry.insert("conn-b", session("s2", "acme"));

        registry.close_connection("conn-a");
        assert!(registry.get("conn-a", "s1").is_none());
        assert!(
            registry.get("conn-b", "s2").is_some(),
            "other connections survive"
        );
    }

    #[test]
    fn deleting_a_session_removes_only_that_session() {
        // ACP's `session/delete`: one session goes, the connection and its
        // other sessions survive.
        let registry = SessionRegistry::new();
        registry.insert("conn-a", session("s1", "acme"));
        registry.insert("conn-a", session("s2", "acme"));

        assert!(registry.remove("conn-a", "s1"));
        assert!(registry.get("conn-a", "s1").is_none());
        assert!(registry.get("conn-a", "s2").is_some());
        assert_eq!(registry.list("conn-a").len(), 1);
    }

    #[test]
    fn removing_a_connections_last_session_prunes_the_connection() {
        // `session/new` + `session/disconnect` over fresh caller-controlled
        // connection ids must not grow the host-wide registry by one empty map
        // per connection, forever.
        let registry = SessionRegistry::new();
        registry.insert("conn-a", session("s1", "acme"));
        registry.insert("conn-b", session("s2", "acme"));

        assert!(registry.remove("conn-a", "s1"));
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
        registry.insert("conn-a", session("s1", "acme"));
        assert!(!registry.remove("conn-a", "ghost"));
        assert!(!registry.remove("conn-b", "s1"));
        assert_eq!(registry.list("conn-a").len(), 1);
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
