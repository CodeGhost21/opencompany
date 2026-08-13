//! The built-in `"operator"` channel adapter.
//!
//! Every company has an operator channel — the human's chat surface. Phase 1
//! backs it with an in-memory buffer: outbound messages the runtime routes here
//! are captured so the HTTP layer (and tests) can read them back. Inbound
//! operator messages arrive as `OperatorMessage` events through the HTTP chat
//! route, not through this stream, so `inbound` is an empty stream for now.

use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use futures::stream::{self, BoxStream};

use crate::Result;
use crate::ports::channel::ChannelAdapter;
use crate::ports::events::EventLog;
use crate::ports::types::{CompanyEvent, CompanyId, EventSeq, InboundMessage, OutboundMessage};

/// The channel id of the always-present operator surface.
pub const OPERATOR_CHANNEL: &str = "operator";

/// A desk-backed [`ChannelAdapter`]. Sending appends an agent reply to the
/// company's durable event log, which is the existing read path for desk chat
/// history. The adapter is deliberately one-per-desk so channel lookup and
/// chat-thread ownership use the same canonical desk id.
#[derive(Clone)]
pub struct DeskChannel {
    company: CompanyId,
    desk_id: String,
    events: Arc<dyn EventLog>,
}

impl DeskChannel {
    /// Creates a channel for an already-resolved desk id.
    pub fn new(company: CompanyId, desk_id: String, events: Arc<dyn EventLog>) -> Self {
        Self {
            company,
            desk_id,
            events,
        }
    }
}

#[async_trait]
impl ChannelAdapter for DeskChannel {
    fn channel_id(&self) -> &str {
        &self.desk_id
    }

    fn inbound(&self) -> BoxStream<'static, InboundMessage> {
        Box::pin(stream::empty())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        self.events
            .append(
                &self.company,
                CompanyEvent::AgentReply {
                    chat_id: self.desk_id.clone(),
                    agent_id: "workflow".to_string(),
                    text: msg.text,
                    steps: msg.steps,
                    task_id: msg.task_id,
                    parent: msg
                        .reply_to
                        .and_then(|reply| reply.chat_id.parse::<u64>().ok())
                        .map(EventSeq::new),
                },
            )
            .await?;
        Ok(())
    }
}

impl std::fmt::Debug for DeskChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeskChannel")
            .field("company", &self.company)
            .field("desk_id", &self.desk_id)
            .finish()
    }
}

/// The built-in operator [`ChannelAdapter`], buffering sent messages in memory.
#[derive(Clone, Default)]
pub struct OperatorChannel {
    sent: Arc<StdMutex<Vec<OutboundMessage>>>,
}

impl OperatorChannel {
    /// Creates an empty operator channel.
    pub fn new() -> Self {
        Self::default()
    }

    /// A snapshot of every message sent on this channel so far.
    pub fn sent(&self) -> Vec<OutboundMessage> {
        self.sent.lock().expect("operator buffer poisoned").clone()
    }
}

#[async_trait]
impl ChannelAdapter for OperatorChannel {
    fn channel_id(&self) -> &str {
        OPERATOR_CHANNEL
    }

    fn inbound(&self) -> BoxStream<'static, InboundMessage> {
        Box::pin(stream::empty())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        self.sent
            .lock()
            .expect("operator buffer poisoned")
            .push(msg);
        Ok(())
    }
}

impl std::fmt::Debug for OperatorChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperatorChannel")
            .field("sent", &self.sent().len())
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn buffers_sent_messages() {
        let channel = OperatorChannel::new();
        assert_eq!(channel.channel_id(), "operator");
        channel
            .send(OutboundMessage {
                message_id: None,
                task_id: None,
                channel: "operator".into(),
                text: "hello".into(),
                steps: Vec::new(),
                reply_to: None,
            })
            .await
            .unwrap();
        assert_eq!(channel.sent().len(), 1);
        assert_eq!(channel.sent()[0].text, "hello");
    }
}
