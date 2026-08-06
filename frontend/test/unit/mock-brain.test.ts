import { spawn, type ChildProcessByStdio } from "node:child_process";
import type { Readable } from "node:stream";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

/**
 * The mock inference backend's decisions (issue #467).
 *
 * # Why this one spawns a process
 *
 * The rest of this directory tests pure functions, and this could have been one
 * too — but the thing that has to be right about `mock-brain.mjs` is the shape
 * it puts on the wire, and that is a property of the server, not of a helper
 * inside it. `src/harness/provider.rs` reads `choices[0].message.tool_calls[]`
 * with `function.arguments` as a JSON *string* and `finish_reason` a sibling of
 * `message`; get any of that wrong and the failure surfaces forty seconds into
 * a browser run as "the reply never arrived". So the subject here is the real
 * server over real HTTP, on an ephemeral port. It costs a few hundred
 * milliseconds once for the whole file.
 *
 * The three arms under test are the three the suite depends on: a plain reply
 * carrying the marker, a scripted tool call, and — the one with a bug worth
 * catching — a directive that must fire exactly once even though the harness
 * resends the whole transcript on every turn.
 */

const SCRIPT = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../e2e/mock-brain.mjs",
);

// `stdio: ["ignore", "ignore", "pipe"]` — only stderr is a stream, which is the
// one this reads the chosen port off.
let server: ChildProcessByStdio<null, null, Readable>;
let origin: string;

/** Starts the server on an ephemeral port and reads back the address it chose. */
beforeAll(async () => {
  server = spawn(process.execPath, [SCRIPT, "--bind", "127.0.0.1:0"], {
    stdio: ["ignore", "ignore", "pipe"],
  });
  origin = await new Promise<string>((ok, fail) => {
    const timer = setTimeout(() => fail(new Error("mock brain never announced a port")), 10_000);
    server.stderr.setEncoding("utf8");
    server.stderr.on("data", (chunk: string) => {
      const found = /listening on (http:\/\/\S+)/.exec(chunk);
      if (found) {
        clearTimeout(timer);
        ok(found[1]);
      }
    });
    server.on("error", fail);
  });
});

afterAll(() => {
  server?.kill();
});

/** POSTs a chat-completions request and returns the parsed reply. */
async function chat(messages: unknown[]): Promise<any> {
  const response = await fetch(`${origin}/v1/chat/completions`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ model: "chat-v1", messages }),
  });
  expect(response.status).toBe(200);
  return response.json();
}

describe("the mock inference backend", () => {
  it("answers an ordinary turn with the marker and no tool call", async () => {
    const reply = await chat([{ role: "user", content: "e2e wiring ping 123" }]);

    expect(reply.choices[0].message.content).toContain("__MOCK_LLM__");
    expect(reply.choices[0].message.tool_calls).toBeUndefined();
    expect(reply.choices[0].finish_reason).toBe("stop");
  });

  it("does not quote the prompt back", async () => {
    // A spec that locates the operator's own bubble by its text must not match
    // the reply as well. The reply is fixed for that reason, and because the
    // harness's wrapping of a prompt is not this server's to predict.
    const reply = await chat([{ role: "user", content: "ship the launch checklist" }]);

    expect(reply.choices[0].message.content).toBe("__MOCK_LLM__ mock inference backend reply.");
  });

  it("calls spawn_task once for a SPAWNONE prompt, with a title off the message", async () => {
    const reply = await chat([{ role: "user", content: "please track this SPAWNONE 456" }]);

    const call = reply.choices[0].message.tool_calls[0];
    expect(call.function.name).toBe("spawn_task");
    // Arguments ride the wire as a JSON string, which is what the host parses.
    expect(JSON.parse(call.function.arguments).title).toContain("SPAWNONE");
    expect(reply.choices[0].finish_reason).toBe("tool_calls");
  });

  it("emits the exact tool call a __MOCK_TOOL_CALL__ directive names", async () => {
    const directive = `__MOCK_TOOL_CALL__ ${JSON.stringify({
      name: "mcp_call_tool",
      arguments: { server: "simple", tool: "echo", arguments: { text: "hi" } },
    })} please`;
    const reply = await chat([{ role: "user", content: directive }]);

    const call = reply.choices[0].message.tool_calls[0];
    expect(call.function.name).toBe("mcp_call_tool");
    // The nested `arguments` object is what the brace scanner has to get right:
    // a naive match would stop at the first closing brace.
    expect(JSON.parse(call.function.arguments)).toEqual({
      server: "simple",
      tool: "echo",
      arguments: { text: "hi" },
    });
  });

  it("serves a directive once, then answers with the tool's own output", async () => {
    // The transcript the harness resends after the tool ran. The directive is
    // still in it; firing again would open a second card per message forever.
    const reply = await chat([
      { role: "user", content: "please track this SPAWNONE 601" },
      { role: "assistant", content: null, tool_calls: [{ id: "c1" }] },
      { role: "tool", tool_call_id: "c1", content: "echo: marker-789" },
    ]);

    expect(reply.choices[0].message.tool_calls).toBeUndefined();
    expect(reply.choices[0].message.content).toContain("__MOCK_LLM__");
    expect(reply.choices[0].message.content).toContain("echo: marker-789");
  });

  it("serves a directive once even when the tool result never comes back", async () => {
    // `spawn_task` is serviced by the runtime's delegation seam rather than the
    // agent's own tool loop, so its result never enters the model-visible
    // transcript: the history looks untouched on the next call of the same
    // turn. Without an identity check the directive fires again, and again,
    // until the loop caps — four cards for one message, which is what the
    // lane's first runs did.
    const history = [{ role: "user", content: "please track this SPAWNONE 800" }];
    const first = await chat(history);
    const second = await chat(history);

    expect(first.choices[0].message.tool_calls[0].function.name).toBe("spawn_task");
    expect(second.choices[0].message.tool_calls).toBeUndefined();
  });

  it("keeps that identity when the same message reaches a second agent", async () => {
    // One operator message reaches the orchestrator and then each desk the turn
    // delegates to, each inside its own wrapper. Keying on anything that
    // includes the wrapper gives every agent a fresh key, and every one of them
    // honours the directive — four cards for one message, which is what the
    // lane's first three runs did.
    const first = await chat([{ role: "user", content: "please track this SPAWNONE 900" }]);
    const second = await chat([
      { role: "user", content: "The operator asked: please track this SPAWNONE 900" },
    ]);

    expect(first.choices[0].message.tool_calls[0].function.name).toBe("spawn_task");
    expect(second.choices[0].message.tool_calls).toBeUndefined();
  });

  it("recognises the dispatcher's tool results, which are a user message", async () => {
    // The shape this host actually produces: OpenHuman's `to_provider_messages`
    // renders a tool result as a *user* turn. Reading only the native `tool`
    // role is what made the lane's first run call `spawn_task` four times for
    // one message, looping until the turn gave up.
    const reply = await chat([
      { role: "user", content: "please track this SPAWNONE 602" },
      { role: "assistant", content: "" },
      {
        role: "user",
        content:
          '[Tool results]\n<tool_result id="mock-call-0">\necho: marker-789\n</tool_result>\n',
      },
    ]);

    expect(reply.choices[0].message.tool_calls).toBeUndefined();
    // Quoted without its wrapper, so the operator's bubble is readable.
    expect(reply.choices[0].message.content).toBe("__MOCK_LLM__ echo: marker-789");
  });

  it("fires a fresh directive even after an earlier one was served", async () => {
    const reply = await chat([
      { role: "user", content: "please track this SPAWNONE 701" },
      { role: "assistant", content: null, tool_calls: [{ id: "c1" }] },
      { role: "tool", tool_call_id: "c1", content: "opened" },
      { role: "assistant", content: "__MOCK_LLM__ opened" },
      { role: "user", content: "and this one SPAWNONE 702" },
    ]);

    expect(reply.choices[0].message.tool_calls[0].function.name).toBe("spawn_task");
  });

  it("returns embeddings at the width the host validates against", async () => {
    const response = await fetch(`${origin}/v1/embeddings`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ model: "embedding-v1", input: ["one", "two"] }),
    });
    const body = await response.json();

    expect(body.data).toHaveLength(2);
    expect(body.data[0].embedding).toHaveLength(1024);
    // Deterministic: two runs of the suite must not disagree about a note.
    expect(body.data[0].embedding).not.toEqual(body.data[1].embedding);
  });
});
