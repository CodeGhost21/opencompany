#![cfg(feature = "openhuman")]
//! **End-to-end proof that a hive-mind desk actually deliberates.**
//!
//! The unit tests in [`hivemind`](opencompany::hivemind) drive the episode
//! driver with a `HiveTurnRunner` that returns strings. They pin the fold, and
//! they cannot tell you whether a *company* deliberates: whether the brain hook
//! fires on an operator message, whether each authorized turn goes through the
//! ordinary harness turn path with its tools and its memory loop, whether the
//! transcript a member is handed is the one the prompt builder promised, or
//! whether the journal ends up holding one row per turn plus one honest closing
//! report.
//!
//! So this boots a **real company** — `RuntimeBuilder`, the embedded OpenHuman
//! harness, the filesystem store, the HTTP surface, loopback magic-link
//! sign-in — and drives it through `POST /api/v1/company/chat`, the same route
//! the console posts to. Only the model is scripted, and the scripted endpoint
//! is **content-aware**: it reads the prompt each agent was handed, works out
//! who is speaking and what that agent can see, and answers accordingly. A
//! member that cites `^N` has to find `N` in the transcript it was given, which
//! is exactly the property a fixed reply queue cannot prove.
//!
//! # What each test proves
//!
//! | Test | Claim |
//! | --- | --- |
//! | `a_desk_deliberates_and_converges_through_the_fold` | three members, one turn each per model call, one `AgentReply` per turn under the right author, one `hive-report` naming the topic and its supporters |
//! | `the_opening_round_is_blind_and_every_later_line_is_attributed` | the first round shows no peer marker line; later rounds render peers as `[seq] <id>: …` and never as the viewer's own words |
//! | `an_objection_silences_an_advocate_and_a_second_topic_carries` | cross-inhibition end to end: the objected author is not among the winning topic's supporters |
//! | `a_desk_reasons_with_what_it_stored_in_an_earlier_episode` | a `memory_store` tool call in episode one is readable by `memory_recall` in episode two, and the room's line cites it |
//! | `a_desk_reasons_with_memory_held_in_a_remote_engine` | the same claim with the memory ports bound to a CortexDB mock: the write lands on `/v1/experience`, the read comes back from `/v1/recall` |
//! | `a_room_that_settles_on_nothing_reports_itself_exhausted` | the budget is spent and the report says so |
//! | `two_carrying_topics_and_no_objection_deadlock` | `Deadlocked`, named honestly |
//! | `a_single_member_desk_answers_with_one_ordinary_turn` | the same company's desk of one is untouched: one reply, no `hive-report` |
//!
//! # Why no shell
//!
//! Every scripted move is a marker line or a memory tool call. Nothing on this
//! path asks for `shell`, so nothing parks for approval and no test depends on
//! an approval policy that would make it hang.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::routing::post;
use serde_json::{Value, json};

use opencompany::CompanyRuntime;
use opencompany::company::CompanyManifest;
use opencompany::hivemind::HIVE_REPORT_AUTHOR;
use opencompany::ports::types::{CompanyEvent, EventSeq};
use opencompany::runtime::{RuntimeBuilder, company_id_from_name};
use opencompany::{AppConfig, AppState};

// ---------------------------------------------------------------------------
// The content-aware scripted model
// ---------------------------------------------------------------------------

/// One request as the script sees it: who is speaking, what they were shown,
/// and what the turn loop has already handed back.
#[derive(Clone, Debug)]
struct Ask {
    /// The teammate this turn belongs to, read out of the prompt's own
    /// `You are @<id>` opening. `None` for a request that is not a hive turn.
    speaker: Option<String>,
    /// The episode prompt this turn was handed, verbatim.
    prompt: String,
    /// Every `tool` message already in this conversation, oldest first.
    tool_outputs: Vec<String>,
    /// The whole message array, for assertions that need the roles.
    messages: Vec<Value>,
}

impl Ask {
    fn read(body: &Value) -> Self {
        let messages: Vec<Value> = body
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // The hive prompt is a user message. The memory loop may prepend a
        // "## Relevant prior work" preamble to it, so it is found by content
        // rather than by position.
        let prompt = messages
            .iter()
            .rev()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
            .filter_map(|message| message.get("content").and_then(Value::as_str))
            .find(|content| content.contains(TRANSCRIPT_HEADING))
            .unwrap_or_default()
            .to_owned();
        let speaker = prompt
            .split_once("You are @")
            .and_then(|(_, rest)| rest.split_once(','))
            .map(|(id, _)| id.trim().to_owned());
        let tool_outputs = messages
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
            .filter_map(|message| message.get("content").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        Self {
            speaker,
            prompt,
            tool_outputs,
            messages,
        }
    }

    /// Who is speaking, or `"?"` — used only to key a script.
    fn who(&self) -> &str {
        self.speaker.as_deref().unwrap_or("?")
    }

    /// Whether this turn is under the blind projection.
    fn blind(&self) -> bool {
        self.prompt.contains("You cannot yet see your peers' positions")
    }

    /// The attributed transcript this turn was handed, as
    /// `(sequence, author, content)`.
    fn transcript(&self) -> Vec<(u64, String, String)> {
        let Some((_, block)) = self.prompt.split_once(TRANSCRIPT_HEADING) else {
            return Vec::new();
        };
        let block = block.split(YOUR_LINE).next().unwrap_or(block);
        block.lines().filter_map(parse_transcript_line).collect()
    }

    /// The sequence of the first transcript line whose text contains `needle`,
    /// which is how a scripted member grounds a citation: it has to read the
    /// number out of what it was shown.
    fn seq_of(&self, needle: &str) -> Option<u64> {
        self.transcript()
            .into_iter()
            .find(|(_, _, content)| content.contains(needle))
            .map(|(seq, _, _)| seq)
    }

    /// The sequence of the first line `author` wrote containing `needle`.
    fn seq_by(&self, author: &str, needle: &str) -> Option<u64> {
        self.transcript()
            .into_iter()
            .find(|(_, who, content)| who == author && content.contains(needle))
            .map(|(seq, _, _)| seq)
    }

    /// Whether this speaker has already said something containing `needle`.
    fn i_said(&self, needle: &str) -> bool {
        let me = self.who().to_owned();
        self.transcript()
            .into_iter()
            .any(|(_, who, content)| who == me && content.contains(needle))
    }
}

/// The heading the prompt builder puts above the attributed transcript.
const TRANSCRIPT_HEADING: &str = "Shared attributed transcript:\n";
/// The closing line of every episode prompt.
const YOUR_LINE: &str = "\n\nYour one line:";
/// The block a member's own previous line is rendered under.
const ALREADY_SAID: &str = "You already said this";

/// `[7] planner: !propose #stage …` → `(7, "planner", "!propose #stage …")`.
fn parse_transcript_line(line: &str) -> Option<(u64, String, String)> {
    let rest = line.strip_prefix('[')?;
    let (seq, rest) = rest.split_once("] ")?;
    let (author, content) = rest.split_once(": ")?;
    Some((
        seq.parse().ok()?,
        author.to_owned(),
        content.trim().to_owned(),
    ))
}

/// What the scripted model does with one request.
#[derive(Clone, Debug)]
enum Reply {
    /// Finish the turn with this assistant text.
    Say(String),
    /// Emit a native `tool_calls` entry with these literal arguments.
    Call { tool: &'static str, args: Value },
}

/// Anything that can answer a request from what it can see in it.
type Responder = Arc<dyn Fn(&Ask) -> Reply + Send + Sync>;

/// A scripted OpenAI-compatible endpoint, served on loopback.
///
/// `/embeddings` is served alongside `/chat/completions` for the same reason
/// `offline_e2e` serves it: the host's embeddings client shares the `base_url`,
/// and a 404 there reads as an inference failure and is not one.
struct Script {
    responder: Responder,
    /// Every request body the harness sent, in order.
    seen: Mutex<Vec<Value>>,
}

impl Script {
    fn bodies(&self) -> Vec<Value> {
        self.seen.lock().expect("script poisoned").clone()
    }

    /// Every request that OPENED a hive turn: its last message is the episode
    /// prompt itself, so a tool round trip inside one turn is not counted twice.
    fn turn_openers(&self) -> Vec<Ask> {
        self.bodies()
            .iter()
            .filter(|body| {
                body.get("messages")
                    .and_then(Value::as_array)
                    .and_then(|messages| messages.last())
                    .is_some_and(|last| {
                        last.get("role").and_then(Value::as_str) == Some("user")
                            && last
                                .get("content")
                                .and_then(Value::as_str)
                                .is_some_and(|content| content.contains(TRANSCRIPT_HEADING))
                    })
            })
            .map(Ask::read)
            .collect()
    }

    /// Every request that carried a hive prompt at all, opener or follow-up.
    fn hive_asks(&self) -> Vec<Ask> {
        self.bodies()
            .iter()
            .map(Ask::read)
            .filter(|ask| ask.speaker.is_some())
            .collect()
    }
}

async fn spawn_script(responder: Responder) -> (String, Arc<Script>) {
    let script = Arc::new(Script {
        responder,
        seen: Mutex::new(Vec::new()),
    });
    let chat = Arc::clone(&script);
    let app = axum::Router::new()
        .route(
            "/chat/completions",
            post(move |Json(body): Json<Value>| {
                let script = Arc::clone(&chat);
                async move {
                    script.seen.lock().expect("script poisoned").push(body.clone());
                    let ask = Ask::read(&body);
                    let message = match (script.responder)(&ask) {
                        Reply::Say(text) => json!({ "role": "assistant", "content": text }),
                        Reply::Call { tool, args } => json!({
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [{
                                "id": format!("call-{tool}"),
                                "type": "function",
                                "function": { "name": tool, "arguments": args.to_string() }
                            }]
                        }),
                    };
                    Json(json!({
                        "choices": [{ "index": 0, "message": message, "finish_reason": "stop" }],
                        "usage": { "prompt_tokens": 12, "completion_tokens": 4 }
                    }))
                }
            }),
        )
        .route(
            "/embeddings",
            post(|Json(_body): Json<Value>| async move {
                Json(json!({
                    "data": [{ "index": 0, "embedding": vec![0.0_f32; 1536] }],
                    "usage": { "prompt_tokens": 1, "total_tokens": 1 }
                }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), script)
}

// ---------------------------------------------------------------------------
// The company
// ---------------------------------------------------------------------------

/// The three-member desk every deliberation test runs on.
const DESK: &str = "lab";
/// The one-member desk that proves the single-responder path is untouched.
const SOLO_DESK: &str = "front";
const THEORIST: &str = "theorist";
const PROGRAMMER: &str = "programmer";
const VERIFIER: &str = "verifier";
const ADMIN: &str = "operator@opencompany.local";

/// A company with one deliberating desk of three and one desk of one.
///
/// `[policy] mode = "full"` so no turn parks: this file is about the room, not
/// the approval gate, and a parked turn would hang the episode rather than fail
/// it. `[tools] allow` is empty on purpose — the memory belt is wired
/// unconditionally by `build_agent`, and nothing else is needed, so no scripted
/// move can reach `shell`.
fn manifest(base_url: &str, hive: &str) -> String {
    format!(
        r#"
[company]
name = "Hive Lab"
summary = "Proves a desk deliberates."

[inference]
provider = "ollama"
base_url = "{base_url}"
model = "llama3"

[policy]
mode = "full"

[tools]
allow = []

[users]
admins = ["{ADMIN}"]

[[agent]]
id = "ceo"
role = "Chief Executive"
tier = "orchestrator"

[[agent]]
id = "{THEORIST}"
role = "Theorist"

[[agent]]
id = "{PROGRAMMER}"
role = "Programmer"

[[agent]]
id = "{VERIFIER}"
role = "Verifier"

[[agent]]
id = "greeter"
role = "Front desk"

[[group_chat]]
id = "{DESK}"
name = "Lab"
description = "Settle hard questions together"
members = ["{THEORIST}", "{PROGRAMMER}", "{VERIFIER}"]
hive = {hive}

[[group_chat]]
id = "{SOLO_DESK}"
name = "Front"
members = ["greeter"]
"#
    )
}

/// Boots the company on loopback and returns its address and live runtime.
async fn boot(
    home: &std::path::Path,
    base_url: &str,
    hive: &str,
    memory: Option<opencompany::store::MemoryOverlay>,
) -> (SocketAddr, Arc<CompanyRuntime>) {
    let mut manifest = CompanyManifest::from_stored_toml(&manifest(base_url, hive))
        .expect("the in-test manifest parses");
    manifest.apply_globals();
    let problems = manifest.validate();
    assert!(problems.is_empty(), "the in-test manifest is valid: {problems:?}");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let company_id = company_id_from_name(&manifest.company.name);
    let state = AppState::new(AppConfig {
        bind: address.to_string(),
        ..AppConfig::default()
    })
    .with_home(home.to_path_buf());
    let mut builder = RuntimeBuilder::new(state.home().to_path_buf(), manifest)
        .with_id(company_id.clone())
        .with_harness(Arc::new(opencompany::harness::HarnessPool::new()));
    if let Some(overlay) = memory {
        builder = builder.with_memory_overlay(&overlay);
    }
    let runtime = Arc::new(builder.build().await.expect("the company builds"));
    state
        .registry()
        .insert(company_id.clone(), Arc::clone(&runtime));
    tokio::spawn(async move {
        let _ = opencompany::server::serve_on(listener, state).await;
    });
    (address, runtime)
}

// ---------------------------------------------------------------------------
// The operator
// ---------------------------------------------------------------------------

/// A cookie-carrying HTTP client — `reqwest`'s own cookie store is behind a
/// feature this crate does not enable.
struct Client {
    inner: reqwest::Client,
    base: String,
    cookie: Mutex<Option<String>>,
}

impl Client {
    fn new(address: SocketAddr) -> Self {
        Self {
            inner: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
            base: format!("http://{address}"),
            cookie: Mutex::new(None),
        }
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let mut request = self.inner.post(format!("{}{path}", self.base)).json(&body);
        if let Some(cookie) = self.cookie.lock().unwrap().clone() {
            request = request.header(reqwest::header::COOKIE, cookie);
        }
        let response = request.send().await.expect("the loopback host answers");
        let status = response.status().as_u16();
        if let Some(set) = response.headers().get(reqwest::header::SET_COOKIE)
            && let Ok(value) = set.to_str()
            && let Some((pair, _)) = value.split_once(';')
        {
            *self.cookie.lock().unwrap() = Some(pair.to_string());
        }
        let text = response.text().await.unwrap_or_default();
        let json = serde_json::from_str(&text).unwrap_or(Value::String(text));
        (status, json)
    }

    /// Signs in over the loopback magic-link flow, which echoes the code.
    async fn sign_in(&self) {
        let (status, body) = self
            .post("/api/v1/company/auth/request", json!({ "email": ADMIN }))
            .await;
        assert_eq!(status, 200, "sign-in refused: {body}");
        let code = body["dev_code"]
            .as_str()
            .unwrap_or_else(|| panic!("no dev_code, so no session: {body}"))
            .to_string();
        let (status, body) = self
            .post("/api/v1/company/auth/verify", json!({ "code": code }))
            .await;
        assert_eq!(status, 200, "the login code was refused: {body}");
    }

    /// Posts one operator message to `desk` and waits for the turn to finish.
    async fn say(&self, desk: &str, text: &str) -> Value {
        let (status, body) = self
            .post(
                "/api/v1/company/chat",
                json!({ "text": text, "chat": desk }),
            )
            .await;
        assert_eq!(status, 200, "chat refused: {body}");
        body
    }
}

// ---------------------------------------------------------------------------
// Reading the journal back
// ---------------------------------------------------------------------------

/// Every `AgentReply` on `chat`, as `(seq, author, text)` in journal order.
async fn replies(runtime: &Arc<CompanyRuntime>, chat: &str) -> Vec<(u64, String, String)> {
    let rows = runtime
        .events()
        .read_from(runtime.id(), EventSeq::new(0), 10_000)
        .await
        .expect("the journal reads back");
    rows.into_iter()
        .filter_map(|stored| match stored.event {
            CompanyEvent::AgentReply {
                chat_id,
                agent_id,
                text,
                ..
            } if chat_id == chat => Some((stored.seq.value(), agent_id, text)),
            _ => None,
        })
        .collect()
}

/// The episode's own turns: every desk reply authored by a seated member.
fn turns(rows: &[(u64, String, String)]) -> Vec<(String, String)> {
    rows.iter()
        .filter(|(_, author, _)| [THEORIST, PROGRAMMER, VERIFIER].contains(&author.as_str()))
        .map(|(_, author, text)| (author.clone(), text.clone()))
        .collect()
}

/// The closing `hive-report` rows, in order.
fn reports(rows: &[(u64, String, String)]) -> Vec<String> {
    rows.iter()
        .filter(|(_, author, _)| author == HIVE_REPORT_AUTHOR)
        .map(|(_, _, text)| text.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// 1 + 2: deliberation converges through the fold, and attribution holds
// ---------------------------------------------------------------------------

/// The topic the room settles on in the convergence tests.
const TOPIC: &str = "answer42";

/// The convergence script.
///
/// It is a state machine over what each speaker can *see*, not a queue: the
/// theorist opens an option, and a peer backs it only once it can read the
/// proposal's sequence number out of the transcript it was handed. A queue
/// would pass whatever the prompt said; this cannot.
fn converging_script() -> Responder {
    Arc::new(|ask: &Ask| {
        if std::env::var("HIVE_DEBUG").is_ok() {
            eprintln!(
                "[ASK] who={} blind={} n={} roles={:?} lastlen={} last80={:?}",
                ask.who(),
                ask.blind(),
                ask.messages.len(),
                ask.messages.iter().map(|m| m.get("role").and_then(Value::as_str).unwrap_or("?").to_owned()).collect::<Vec<_>>(),
                ask.messages.last().and_then(|m| m.get("content")).and_then(Value::as_str).map(str::len).unwrap_or(0),
                ask.messages.last().and_then(|m| m.get("content")).and_then(Value::as_str).map(|c| c.chars().take(90).collect::<String>()).unwrap_or_default()
            );
        }
        let propose = format!("!propose #{TOPIC} The closed form of the recurrence is 42.");
        // A citation is only available once the proposal is visible. In the
        // blind round it is not, which is exactly what the blind round means.
        let grounds = ask.seq_of(&format!("!propose #{TOPIC}"));
        let line = match (ask.who(), grounds) {
            (THEORIST, None) => propose,
            (_, None) => "!question I need the opening position before I can back anything."
                .to_owned(),
            (who, Some(seq)) if ask.prompt.contains("The room has reached quorum") => {
                format!("!commit #{TOPIC} ^{seq} {who} records the room's decision.")
            }
            (THEORIST, Some(seq)) => {
                format!("!evidence #{TOPIC} ^{seq} The recurrence closes at 42 for every base case.")
            }
            (who, Some(seq)) if !ask.i_said("!support") => {
                format!("!support #{TOPIC} ^{seq} {who} checked the derivation and it holds.")
            }
            (_, Some(_)) => "!question Nothing further from me until somebody else moves."
                .to_owned(),
        };
        Reply::Say(line)
    })
}

/// A desk of three that must all back a topic with grounds before it carries.
const UNANIMOUS: &str = "{ enabled = true, turn_budget = 12, quorum = 3, blind_round = true }";

/// **Deliberation converges through the fold.**
///
/// One operator message, three teammates, and an outcome the room can name.
#[tokio::test]
async fn a_desk_deliberates_and_converges_through_the_fold() {
    let home = tempfile::tempdir().unwrap();
    let (base_url, script) = spawn_script(converging_script()).await;
    let (address, runtime) = boot(home.path(), &base_url, UNANIMOUS, None).await;
    let client = Client::new(address);
    client.sign_in().await;

    client.say(DESK, "Settle the closed form of the recurrence.").await;

    let rows = replies(&runtime, DESK).await;
    let turns = turns(&rows);
    assert!(
        turns.len() >= 3,
        "a room of three takes at least one turn each: {rows:?}"
    );
    // Every deliberation row is authored by the teammate that took the turn,
    // and carries the ONE line the room counts — never a paragraph.
    for (author, text) in &turns {
        assert!(
            [THEORIST, PROGRAMMER, VERIFIER].contains(&author.as_str()),
            "{rows:?}"
        );
        assert!(text.starts_with('!'), "not a marker line: {text}");
        assert!(!text.contains('\n'), "more than one line: {text}");
    }

    // One closing row, under the reserved author, naming the topic and every
    // member whose grounded support carried it.
    let reports = reports(&rows);
    assert_eq!(reports.len(), 1, "exactly one closing row: {rows:?}");
    let report = &reports[0];
    assert!(report.contains(&format!("#{TOPIC}")), "{report}");
    assert!(report.contains("settled on"), "{report}");
    for member in [THEORIST, PROGRAMMER, VERIFIER] {
        assert!(
            report.contains(member),
            "the room needed all three to carry #{TOPIC}, so all three are named: {report}"
        );
    }

    // No single-responder bubble: the desk's only authored rows are the
    // episode's own turns and its report. In particular the orchestrator never
    // answered on top of the room.
    let strays: Vec<_> = rows
        .iter()
        .filter(|(_, author, _)| {
            author != HIVE_REPORT_AUTHOR
                && !["theorist", "programmer", "verifier"].contains(&author.as_str())
                && ["ceo", "greeter"].contains(&author.as_str())
        })
        .collect();
    assert!(strays.is_empty(), "a second responder answered too: {strays:?}");

    // One model call per turn, and the calls are the turns: the speakers the
    // endpoint was asked for are exactly the authors the journal recorded, in
    // order.
    let openers = script.turn_openers();
    let asked: Vec<String> = openers.iter().map(|ask| ask.who().to_owned()).collect();
    let journaled: Vec<String> = turns.iter().map(|(author, _)| author.clone()).collect();
    assert_eq!(
        asked, journaled,
        "one model call per journaled turn, in the same order"
    );
    assert_eq!(
        script.hive_asks().len(),
        openers.len(),
        "no hive turn needed a second model call: nothing on this path uses a tool"
    );
}

/// **Attribution and the blind round.**
///
/// Asserted from the captured request bodies, which is the only place the
/// claim actually lives: the journal cannot tell you what a member was *shown*.
#[tokio::test]
async fn the_opening_round_is_blind_and_every_later_line_is_attributed() {
    let home = tempfile::tempdir().unwrap();
    let (base_url, script) = spawn_script(converging_script()).await;
    let (address, _runtime) = boot(home.path(), &base_url, UNANIMOUS, None).await;
    let client = Client::new(address);
    client.sign_in().await;

    client.say(DESK, "Settle the closed form of the recurrence.").await;

    let openers = script.turn_openers();
    assert!(openers.len() >= 4, "a blind round plus at least one open turn");

    let blind: Vec<&Ask> = openers.iter().filter(|ask| ask.blind()).collect();
    assert_eq!(
        blind.len(),
        3,
        "the opening round is one blind turn per member"
    );
    for ask in &blind {
        let me = ask.who().to_owned();
        let peers: Vec<_> = ask
            .transcript()
            .into_iter()
            .filter(|(_, author, _)| {
                author != &me && [THEORIST, PROGRAMMER, VERIFIER].contains(&author.as_str())
            })
            .collect();
        assert!(
            peers.is_empty(),
            "a peer's position leaked into a blind turn for @{me}: {peers:?}"
        );
    }

    // Later turns see the room, attributed by id, with the sequence that makes
    // the line citable.
    let seeing: Vec<&Ask> = openers.iter().filter(|ask| !ask.blind()).collect();
    assert!(!seeing.is_empty(), "the blind round is not the whole episode");
    let mut saw_attributed_peer = false;
    let mut saw_own_line = false;
    for ask in &seeing {
        let me = ask.who().to_owned();
        for (seq, author, content) in ask.transcript() {
            if author == me || !["theorist", "programmer", "verifier"].contains(&author.as_str()) {
                continue;
            }
            saw_attributed_peer = true;
            // Rendered exactly as `[seq] <peer id>: …`.
            assert!(
                ask.prompt.contains(&format!("[{seq}] {author}: {content}")),
                "a peer's line is not attributed the way the citation grammar needs: {content}"
            );
            // And never presented as this viewer's own words — not in the
            // assistant role, and not under the "you already said this" block.
            for message in &ask.messages {
                if message.get("role").and_then(Value::as_str) == Some("assistant")
                    && let Some(text) = message.get("content").and_then(Value::as_str)
                {
                    assert!(
                        !text.contains(&content),
                        "@{me} was handed @{author}'s line as its own assistant turn: {text}"
                    );
                }
            }
            if let Some((_, own)) = ask.prompt.split_once(ALREADY_SAID) {
                let own = own.split(TRANSCRIPT_HEADING).next().unwrap_or(own);
                assert!(
                    !own.contains(&content),
                    "@{author}'s line was rendered to @{me} as something @{me} had said"
                );
            }
        }
        // A member that has spoken is shown its own last line, so it does not
        // restate it.
        if let Some((_, mine)) = ask.transcript().into_iter().rev().find_map(|(_, a, c)| {
            (a == me).then_some((a, c))
        }) {
            saw_own_line = true;
            let block = ask
                .prompt
                .split_once(ALREADY_SAID)
                .map(|(_, rest)| rest.split(TRANSCRIPT_HEADING).next().unwrap_or(rest).to_owned())
                .unwrap_or_default();
            assert!(
                block.contains(&mine),
                "@{me} was not shown its own last line: {block}"
            );
        }
    }
    assert!(saw_attributed_peer, "no open turn ever saw a peer");
    assert!(saw_own_line, "no open turn was shown its own previous line");
}
