#!/usr/bin/env node
//
// The mock inference backend the live-brain end-to-end lane runs against
// (issue #467).
//
// Four of the suite's specs need an agent that actually executes, which needs
// a host built with `--features openhuman,tinycortex,mcp` **and** something for
// that harness to think with. This is that something: an OpenAI-compatible
// chat-completions endpoint with no model behind it, whose answers are a pure
// function of the prompt. `wiring.spec.ts`'s header has described it since the
// day it was written ("a mocked inference backend that echoes a `__MOCK_LLM__`
// marker"); until now nobody had committed one, so the specs it describes were
// skipped rather than run.
//
// # Why a mock and not a real provider
//
// A real backend would make the suite depend on a credential, a network, and a
// model's mood. The specs behind `PW_LIVE_BRAIN` do not assert anything
// about the quality of a reply — they assert that the chain *runs*: session →
// console → `POST /company/chat` → harness → inference → tool call → board card
// → journal → rendered bubble. Every link in that chain is real here. Only the
// cognition is scripted, because scripted cognition is the only kind a test can
// assert on.
//
// # The wire format
//
// `src/harness/provider.rs`'s `HostedProvider` POSTs to
// `{base_url}/chat/completions` and parses `choices[0].message.{content,
// tool_calls}` plus `choices[0].finish_reason` — plain OpenAI. The host's
// embeddings client (`src/harness/embeddings.rs`) shares the same base URL and
// POSTs to `{base_url}/embeddings`, and it *validates* the returned width, so
// `/embeddings` is served here too rather than left to 404 in the middle of a
// memory write.
//
// # The three arms
//
// Everything this server does is decided by scanning the request's messages:
//
//   1. a message carrying `__MOCK_TOOL_CALL__ {"name":…,"arguments":{…}}` —
//      emit exactly that tool call, once. `mcp.spec.ts` uses it to make an
//      agent call a named MCP tool without a model that might decide not to.
//   2. a message carrying `SPAWNONE` — call `spawn_task` once, which is what
//      `chat-to-card.spec.ts` needs an orchestrator to do.
//   3. anything else — a plain text reply carrying the `__MOCK_LLM__` marker.
//
// "Once" is load-bearing and is why arms 1 and 2 look at position rather than
// mere presence. The harness sends the whole thread history on every turn, so a
// directive an earlier turn already served is still in the transcript on the
// next one; re-firing it would open a second card per message forever. A
// directive counts as served when a tool result — or an assistant turn carrying
// tool calls — appears *after* it. A fresh directive is always the last one, so
// it always fires.
//
// The plain reply deliberately does NOT echo the operator's prompt back. A spec
// that locates the operator's own bubble by its text (`chat-to-card.spec.ts`
// does) would find two matches if the reply quoted it. The one thing that is
// echoed is a tool *result*, because `mcp.spec.ts` asserts the remote tool's
// output reached the agent, and the agent's bubble is where it can see it.
//
// # Running it
//
// `playwright.config.ts` starts this as a `webServer` when `PW_LIVE_BRAIN=1`
// and it is managing the host, so `npm run e2e:live` is the whole command. It
// is a standalone script with no dependencies for the other case: if you
// brought your own host with `PW_BASE_URL`, run
//
//     node frontend/test/e2e/mock-brain.mjs --bind 127.0.0.1:8099
//
// and point that host's `OPENCOMPANY_INFERENCE_URL` at `…:8099/v1` with any
// non-empty `OPENCOMPANY_INFERENCE_KEY` (nothing here checks the bearer; the
// host needs one only because a credential is what makes it choose a live
// harness over the offline echo brain).
//
// Usage:
//   node mock-brain.mjs [--bind HOST:PORT]
// Env:
//   MOCK_BRAIN_BIND   same as --bind (default 127.0.0.1:8099)
//
// A `:0` port binds an ephemeral one; the chosen address is always printed to
// stderr as `[mock brain] listening on http://HOST:PORT`, which is how
// `test/unit/mock-brain.test.ts` finds the server it just spawned.

import { createServer } from "node:http";

/** The marker every text reply carries, so a spec can prove the reply is ours. */
const MARKER = "__MOCK_LLM__";

/** Prefix of the "call exactly this tool" directive, followed by a JSON object. */
const TOOL_CALL_DIRECTIVE = "__MOCK_TOOL_CALL__";

/** The cue that makes the orchestrator open exactly one board card. */
const SPAWN_DIRECTIVE = "SPAWNONE";

/**
 * Width of every vector `/embeddings` returns. `HostedEmbeddings` compares this
 * against its declared dimensionality and errors on a mismatch rather than
 * truncating, and its default is 1024 (`embedding-v1`'s only allowed size).
 */
const EMBEDDING_DIM = 1024;

/** How much of a tool result is quoted back in the reply that follows it. */
const TOOL_ECHO_LIMIT = 2000;

/**
 * The text of one wire message, tolerating both shapes OpenAI allows: a plain
 * string, and the content-part array. The host only ever sends the former;
 * the latter costs two lines and removes a way for this to go quietly wrong.
 *
 * @param {any} message
 * @returns {string}
 */
function textOf(message) {
  const content = message?.content;
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((part) => (typeof part?.text === "string" ? part.text : ""))
      .join(" ");
  }
  return "";
}

/**
 * Reads a complete JSON object out of `text` starting at the first `{` at or
 * after `from`, by counting braces outside of string literals.
 *
 * A regex cannot do this: the directive's payload nests (`{"name":…,
 * "arguments":{…}}`) and is followed by whatever prose the harness wrapped the
 * operator's message in, so there is no delimiter to match against — only
 * balance.
 *
 * @param {string} text
 * @param {number} from
 * @returns {any | null} the parsed value, or null if nothing balanced parses
 */
function readJsonObject(text, from) {
  const start = text.indexOf("{", from);
  if (start < 0) return null;

  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let i = start; i < text.length; i += 1) {
    const ch = text[i];
    if (inString) {
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') inString = true;
    else if (ch === "{") depth += 1;
    else if (ch === "}") {
      depth -= 1;
      if (depth === 0) {
        try {
          return JSON.parse(text.slice(start, i + 1));
        } catch {
          return null;
        }
      }
    }
  }
  return null;
}

/**
 * The line `needle` sits on, collapsed and clipped — a readable title for the
 * card `SPAWNONE` opens, so a failed run shows which message opened it.
 *
 * @param {string} text
 * @param {string} needle
 * @returns {string}
 */
function titleFrom(text, needle) {
  const at = text.indexOf(needle);
  if (at < 0) return "Mock spawned task";
  const lineStart = text.lastIndexOf("\n", at) + 1;
  const lineEnd = text.indexOf("\n", at);
  const line = text.slice(lineStart, lineEnd < 0 ? text.length : lineEnd);
  const collapsed = line.replace(/\s+/g, " ").trim();
  if (!collapsed) return "Mock spawned task";
  return collapsed.length > 80 ? `${collapsed.slice(0, 77)}...` : collapsed;
}

/**
 * The last directive in the thread, or null. Returns its position too, because
 * "has this already been served" is a question about what follows it.
 *
 * @param {any[]} messages
 * @returns {{index: number, name: string, arguments: any} | null}
 */
function findDirective(messages) {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const text = textOf(messages[i]);
    const at = text.indexOf(TOOL_CALL_DIRECTIVE);
    if (at >= 0) {
      const payload = readJsonObject(text, at + TOOL_CALL_DIRECTIVE.length);
      if (payload && typeof payload.name === "string") {
        return {
          index: i,
          name: payload.name,
          arguments: payload.arguments ?? {},
        };
      }
      // A malformed payload is a broken spec, not a plain turn. Say so loudly
      // rather than answering with text the spec will fail on obscurely.
      process.stderr.write(
        `[mock brain] ${TOOL_CALL_DIRECTIVE} found but its JSON payload did not parse\n`,
      );
      return null;
    }
    if (text.includes(SPAWN_DIRECTIVE)) {
      return {
        index: i,
        name: "spawn_task",
        arguments: { title: titleFrom(text, SPAWN_DIRECTIVE) },
      };
    }
  }
  return null;
}

/**
 * Whether the directive at `index` has already been acted on in this thread:
 * a tool result, or an assistant turn carrying tool calls, after it.
 *
 * @param {any[]} messages
 * @param {number} index
 * @returns {boolean}
 */
function alreadyServed(messages, index) {
  return messages.slice(index + 1).some((message) => {
    if (message?.role === "tool") return true;
    return (
      message?.role === "assistant" &&
      Array.isArray(message?.tool_calls) &&
      message.tool_calls.length > 0
    );
  });
}

/**
 * The reply body for one chat-completions request.
 *
 * @param {any} body the parsed request
 * @returns {any} an OpenAI-shaped chat completion
 */
function chatCompletion(body) {
  const messages = Array.isArray(body?.messages) ? body.messages : [];
  const model = typeof body?.model === "string" ? body.model : "mock-brain";
  const directive = findDirective(messages);

  if (directive && !alreadyServed(messages, directive.index)) {
    process.stderr.write(`[mock brain] tool call: ${directive.name}\n`);
    return completion(model, {
      role: "assistant",
      content: null,
      tool_calls: [
        {
          id: `mock-call-${directive.index}`,
          type: "function",
          function: {
            name: directive.name,
            arguments: JSON.stringify(directive.arguments),
          },
        },
      ],
    }, "tool_calls");
  }

  const last = messages[messages.length - 1];
  const content =
    last?.role === "tool"
      ? `${MARKER} ${textOf(last).slice(0, TOOL_ECHO_LIMIT)}`
      : `${MARKER} mock inference backend reply.`;
  process.stderr.write(`[mock brain] text reply (${content.length} chars)\n`);
  return completion(model, { role: "assistant", content }, "stop");
}

/**
 * Wraps one assistant message in the completion envelope, with a zeroed usage
 * block. Zero is the honest number and it keeps the harness's cost pipeline on
 * its billing-free path, so a suite run never books spend against the company.
 *
 * @param {string} model
 * @param {any} message
 * @param {string} finishReason
 * @returns {any}
 */
function completion(model, message, finishReason) {
  return {
    id: "chatcmpl-mock",
    object: "chat.completion",
    created: 0,
    model,
    choices: [{ index: 0, message, finish_reason: finishReason }],
    usage: { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 },
  };
}

/**
 * A deterministic unit-ish vector for one input. Never random: two runs of the
 * suite must not disagree about what a note means.
 *
 * @param {string} input
 * @returns {number[]}
 */
function embedding(input) {
  let seed = 2166136261;
  for (let i = 0; i < input.length; i += 1) {
    seed = Math.imul(seed ^ input.charCodeAt(i), 16777619) >>> 0;
  }
  const vector = new Array(EMBEDDING_DIM);
  for (let i = 0; i < EMBEDDING_DIM; i += 1) {
    seed = (Math.imul(seed, 1103515245) + 12345) >>> 0;
    vector[i] = seed / 4294967295 - 0.5;
  }
  return vector;
}

/**
 * The embeddings reply for one request, in input order.
 *
 * @param {any} body
 * @returns {any}
 */
function embeddings(body) {
  const raw = body?.input;
  const inputs = Array.isArray(raw) ? raw : [raw ?? ""];
  return {
    object: "list",
    model: typeof body?.model === "string" ? body.model : "mock-embedding",
    data: inputs.map((input, index) => ({
      object: "embedding",
      index,
      embedding: embedding(String(input)),
    })),
    usage: { prompt_tokens: 0, total_tokens: 0 },
  };
}

/**
 * Reads a whole request body.
 *
 * @param {import("node:http").IncomingMessage} request
 * @returns {Promise<string>}
 */
function readBody(request) {
  return new Promise((resolve, reject) => {
    /** @type {Buffer[]} */
    const chunks = [];
    request.on("data", (chunk) => chunks.push(chunk));
    request.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    request.on("error", reject);
  });
}

/**
 * @param {import("node:http").ServerResponse} response
 * @param {number} status
 * @param {any} payload
 */
function sendJson(response, status, payload) {
  const body = JSON.stringify(payload);
  response.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(body),
  });
  response.end(body);
}

const server = createServer((request, response) => {
  const path = new URL(request.url ?? "/", "http://localhost").pathname;

  // Whatever `{base_url}` the host was given, the two routes it POSTs are
  // `…/chat/completions` and `…/embeddings`. Matching on the suffix means a
  // base URL with or without a `/v1` both work, which is one fewer way for the
  // lane's configuration and this server to disagree.
  if (path === "/healthz") {
    sendJson(response, 200, { ok: true });
    return;
  }

  if (request.method !== "POST") {
    sendJson(response, 405, { error: `${request.method} is not served here` });
    return;
  }

  void readBody(request)
    .then((raw) => {
      /** @type {any} */
      let body;
      try {
        body = raw ? JSON.parse(raw) : {};
      } catch (error) {
        sendJson(response, 400, { error: `unparseable request body: ${error}` });
        return;
      }
      if (path.endsWith("/chat/completions")) {
        sendJson(response, 200, chatCompletion(body));
      } else if (path.endsWith("/embeddings")) {
        sendJson(response, 200, embeddings(body));
      } else {
        sendJson(response, 404, { error: `no mock route for ${path}` });
      }
    })
    .catch((error) => {
      sendJson(response, 500, { error: String(error) });
    });
});

const bindArgument = process.argv.indexOf("--bind");
const bind =
  (bindArgument >= 0 ? process.argv[bindArgument + 1] : undefined) ||
  process.env.MOCK_BRAIN_BIND ||
  "127.0.0.1:8099";
const separator = bind.lastIndexOf(":");
const host = separator > 0 ? bind.slice(0, separator) : "127.0.0.1";
const port = Number(separator > 0 ? bind.slice(separator + 1) : bind);

server.on("error", (error) => {
  process.stderr.write(`[mock brain] cannot bind ${bind}: ${error}\n`);
  process.exit(1);
});

server.listen(port, host, () => {
  const address = server.address();
  const chosen = typeof address === "object" && address ? address.port : port;
  process.stderr.write(`[mock brain] listening on http://${host}:${chosen}\n`);
});
