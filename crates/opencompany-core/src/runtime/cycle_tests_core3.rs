pub(super) use super::*;
pub(super) use crate::ports::tasks::TaskTitle;
pub(super) use std::sync::Arc;
pub(super) use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) use crate::company::CompanyManifest;
pub(super) use crate::company::runtime::CompanyMail;
pub(super) use crate::policy::ManifestApprovalGate;
pub(super) use crate::ports::ChannelAdapter;
pub(super) use crate::ports::brain::Brain;
pub(super) use crate::ports::types::SecretValue;
pub(super) use crate::ports::types::{
    ActorKind, ChunkAddr, ChunkHit, ChunkMeta, CompressedTrace, ContextChunk, CycleResult,
    EffectGroup, EventSeq, EvictionPolicy, ReplyTo, TaskResult, TokenUsage,
};
pub(super) use crate::ports::{ContextStore, MemoryStore};
pub(super) use crate::runtime::RuntimeBuilder;
pub(super) use crate::runtime::channel::OperatorChannel;
pub(super) use crate::server::ops::mailer::RecordingMailSender;
pub(super) use crate::server::ops::smtp::{SmtpCredentials, SmtpSecurity};
pub(super) use crate::store::paths::Bundle;
pub(super) use crate::store::{FsContextStore, FsMemoryStore};

/* ---- issue #1890 E: the thread index ---- */

pub(super) fn op(
    seq: u64,
    chat: &str,
    parent: Option<u64>,
    text: &str,
) -> crate::ports::types::StoredEvent {
    crate::ports::types::StoredEvent {
        seq: EventSeq::new(seq),
        company: CompanyId::new("acme"),
        event: CompanyEvent::OperatorMessage {
            text: text.to_string(),
            by: None,
            chat: Some(chat.to_string()),
            parent: parent.map(EventSeq::new),
            deliverable: None,
            mentions: Vec::new(),
            attachments: Vec::new(),
        },
        at_millis: seq,
    }
}

pub(super) fn agent_reply(seq: u64, chat: &str, parent: u64) -> crate::ports::types::StoredEvent {
    crate::ports::types::StoredEvent {
        seq: EventSeq::new(seq),
        company: CompanyId::new("acme"),
        event: CompanyEvent::AgentReply {
            audience: Vec::new(),
            chat_id: chat.to_string(),
            agent_id: "ceo".to_string(),
            text: "an answer".to_string(),
            steps: Vec::new(),
            task_id: None,
            outputs: Vec::new(),
            parent: Some(EventSeq::new(parent)),
            mentions: Vec::new(),
            mention_depth: 0,
        },
        at_millis: seq,
    }
}

/// A manifest whose desk id and display name are different strings — the
/// only shape in which an alias bug is visible at all.
pub(super) fn manifest_with_named_desk() -> CompanyManifest {
    toml::from_str(
        r#"
        [company]
        name = "Acme"

        [[agent]]
        id = "ceo"
        role = "Chief"

        [policy]
        mode = "supervised"

        [[group_chat]]
        id = "growth_desk"
        name = "Growth"
        "#,
    )
    .expect("parse manifest")
}
