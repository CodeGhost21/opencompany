//! Reassembling `text/event-stream` frames from a byte stream.
//!
//! The browser gets this for free from `EventSource`. The desktop cannot use
//! `EventSource` at all — it cannot set a request header, so it cannot carry
//! the session header a paired device authenticates with, and a `SameSite=Lax`
//! cookie is never sent cross-site either. So the desktop reads the stream with
//! an ordinary HTTP client, which means parsing the wire format by hand.
//!
//! Deliberately a *parser over chunks*, not over lines: a TCP read boundary
//! falls wherever it falls, and it will eventually fall in the middle of a JSON
//! payload. A line-oriented reader that assumed each chunk was whole would work
//! for months and then drop one event under load, which is the kind of bug
//! nobody traces back to here.
//!
//! Only what the console consumes is implemented. `event:` names, `id:` and
//! `retry:` are parsed far enough to be *ignored correctly* — the host emits
//! only default-type messages, and silently treating a named event as a default
//! one would be worse than dropping it.

/// Accumulates bytes and yields complete event payloads.
#[derive(Debug, Default)]
pub struct SseDecoder {
    /// Bytes received but not yet forming a complete event.
    buffer: String,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds a chunk, returning every event completed by it.
    ///
    /// A chunk may complete several events, part of one, or none.
    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buffer.push_str(chunk);
        let mut out = Vec::new();

        // An event ends at a blank line. `\r\n` is legal in the format and some
        // proxies rewrite line endings, so both are accepted.
        while let Some((frame, rest)) = split_frame(&self.buffer) {
            if let Some(data) = decode_frame(&frame) {
                out.push(data);
            }
            self.buffer = rest;
        }
        out
    }
}

/// Splits the first complete frame off `buffer`, if there is one.
fn split_frame(buffer: &str) -> Option<(String, String)> {
    let candidates = ["\r\n\r\n", "\n\n", "\r\r"];
    let (index, terminator) = candidates
        .iter()
        .filter_map(|t| buffer.find(t).map(|i| (i, *t)))
        .min_by_key(|(i, _)| *i)?;
    let frame = buffer[..index].to_string();
    let rest = buffer[index + terminator.len()..].to_string();
    Some((frame, rest))
}

/// The `data` payload of one frame, or `None` when there is nothing to deliver.
fn decode_frame(frame: &str) -> Option<String> {
    let mut data: Vec<&str> = Vec::new();
    let mut named_event: Option<&str> = None;

    for line in frame.split(['\n', '\r']).filter(|l| !l.is_empty()) {
        // A line starting with `:` is a comment. Hosts send these as keep-alive
        // pings, and treating one as data would hand the console a payload it
        // cannot parse on every heartbeat.
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            // A bare field name with no colon is a field with an empty value.
            None => (line, ""),
        };
        match field {
            "data" => data.push(value),
            "event" => named_event = Some(value),
            // `id` and `retry` are meaningful to a reconnecting client. This one
            // does not resume by `Last-Event-ID` — the console's poll is the
            // safety net — so they are read and dropped rather than
            // misinterpreted.
            _ => {}
        }
    }

    // The console subscribes to default-type messages only, which is what
    // `EventSource.onmessage` delivers. Passing a named event through here would
    // silently promote it to a message the caller never asked for.
    if named_event.is_some_and(|name| name != "message") {
        return None;
    }
    if data.is_empty() {
        return None;
    }
    Some(data.join("\n"))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_whole_event_in_one_chunk_is_delivered() {
        let mut decoder = SseDecoder::new();
        assert_eq!(
            decoder.push("data: {\"type\":\"agent_reply\"}\n\n"),
            vec!["{\"type\":\"agent_reply\"}"]
        );
    }

    #[test]
    fn an_event_split_across_chunks_is_reassembled() {
        // THE reason this is a chunk decoder. A read boundary lands wherever
        // the network puts it, and eventually that is mid-payload. A
        // line-oriented reader works until it doesn't, under load, once.
        let mut decoder = SseDecoder::new();
        assert!(decoder.push("data: {\"ty").is_empty());
        assert!(decoder.push("pe\":\"tool_call\"}").is_empty());
        assert_eq!(decoder.push("\n\n"), vec!["{\"type\":\"tool_call\"}"]);
    }

    #[test]
    fn several_events_in_one_chunk_all_arrive() {
        let mut decoder = SseDecoder::new();
        assert_eq!(
            decoder.push("data: one\n\ndata: two\n\n"),
            vec!["one", "two"]
        );
    }

    #[test]
    fn keep_alive_comments_are_not_delivered_as_data() {
        // Hosts ping with a bare comment to hold the connection open. Handing
        // that to the console as a message would make it try to `JSON.parse` a
        // heartbeat on every tick.
        let mut decoder = SseDecoder::new();
        assert!(decoder.push(": keep-alive\n\n").is_empty());
        assert_eq!(decoder.push("data: real\n\n"), vec!["real"]);
    }

    #[test]
    fn crlf_line_endings_parse_the_same() {
        let mut decoder = SseDecoder::new();
        assert_eq!(decoder.push("data: windows\r\n\r\n"), vec!["windows"]);
    }

    #[test]
    fn multi_line_data_joins_with_newlines() {
        // The format's own rule. A payload containing a newline arrives as two
        // `data:` lines and has to be rejoined, or the JSON is truncated.
        let mut decoder = SseDecoder::new();
        assert_eq!(
            decoder.push("data: {\ndata: \"a\": 1}\n\n"),
            vec!["{\n\"a\": 1}"]
        );
    }

    #[test]
    fn a_named_event_is_dropped_rather_than_promoted() {
        let mut decoder = SseDecoder::new();
        assert!(decoder.push("event: ping\ndata: nope\n\n").is_empty());
        // An explicit `message` is the default type, so it does arrive.
        assert_eq!(decoder.push("event: message\ndata: yes\n\n"), vec!["yes"]);
    }

    #[test]
    fn id_and_retry_fields_do_not_become_data() {
        let mut decoder = SseDecoder::new();
        assert_eq!(
            decoder.push("id: 42\nretry: 1000\ndata: payload\n\n"),
            vec!["payload"]
        );
    }

    #[test]
    fn a_frame_with_no_data_yields_nothing() {
        let mut decoder = SseDecoder::new();
        assert!(decoder.push("id: 7\n\n").is_empty());
    }

    #[test]
    fn a_value_keeps_its_internal_colons() {
        // JSON is full of colons; splitting on all of them would truncate every
        // payload after the first key.
        let mut decoder = SseDecoder::new();
        assert_eq!(
            decoder.push("data: {\"url\":\"https://x.test/a\"}\n\n"),
            vec!["{\"url\":\"https://x.test/a\"}"]
        );
    }
}
