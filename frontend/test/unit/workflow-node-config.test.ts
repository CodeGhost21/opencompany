import { describe, expect, it } from "vitest";

import {
  RESERVED_CONFIG_KEYS,
  configDraftFrom,
  configDraftProblem,
  configFieldProblem,
  configFieldSpecs,
  configFromDraft,
  hasConfigForm,
} from "@/lib/workflow-node-config";

/**
 * The five withheld node kinds' config forms (issue #541). The engine
 * (`vendor/openhuman/vendor/tinyflows`) reads exact keys off a node's `config`;
 * these helpers turn form strings into those keys and back. Getting a key
 * wrong ships a node that errors at run time, so the contract is pinned here:
 *
 * - each kind's valid draft serializes to EXACTLY the engine keys, empties
 *   omitted;
 * - a saved node's config round-trips through hydrate → serialize;
 * - an UNKNOWN config key (an orchestrator-authored `connection_ref`, a
 *   sub_workflow's `execution`) is PRESERVED — the anti-data-loss guard;
 * - malformed JSON and missing required fields are caught before submit;
 * - a sub_workflow can't call itself;
 * - the output NEVER carries a host-reserved key.
 */

describe("hasConfigForm", () => {
  it("covers exactly the five withheld kinds", () => {
    for (const kind of [
      "tool_call",
      "http_request",
      "switch",
      "output_parser",
      "sub_workflow",
    ]) {
      expect(hasConfigForm(kind), kind).toBe(true);
    }
    for (const kind of ["trigger", "agent", "condition", "merge", "transform", "output"]) {
      expect(hasConfigForm(kind), kind).toBe(false);
      expect(configFieldSpecs(kind)).toEqual([]);
    }
  });
});

describe("configFromDraft — valid drafts emit exactly the engine keys", () => {
  it("tool_call: slug + parsed args, empties omitted", () => {
    expect(
      configFromDraft("tool_call", { slug: "web_search", args: '{ "query": "hi" }' }),
    ).toEqual({ slug: "web_search", args: { query: "hi" } });

    // No args → the key is omitted entirely, not sent as "".
    expect(configFromDraft("tool_call", { slug: "web_search", args: "" })).toEqual({
      slug: "web_search",
    });
  });

  it("http_request: method + url + parsed headers/body", () => {
    expect(
      configFromDraft("http_request", {
        method: "POST",
        url: "https://api.example.com",
        headers: '{ "Authorization": "Bearer x" }',
        body: '{ "hello": "world" }',
      }),
    ).toEqual({
      method: "POST",
      url: "https://api.example.com",
      headers: { Authorization: "Bearer x" },
      body: { hello: "world" },
    });

    // A JSON string body is a valid body — it stays a string.
    expect(
      configFromDraft("http_request", { method: "GET", url: "u", body: '"raw text"' }),
    ).toEqual({ method: "GET", url: "u", body: "raw text" });
  });

  it("switch: field or expression, whichever is filled", () => {
    expect(configFromDraft("switch", { field: "status", expression: "" })).toEqual({
      field: "status",
    });
    expect(configFromDraft("switch", { field: "", expression: "=item.score > 0.5" })).toEqual({
      expression: "=item.score > 0.5",
    });
  });

  it("output_parser: parsed schema + boolean auto_fix, default omitted", () => {
    expect(
      configFromDraft("output_parser", {
        schema: '{ "type": "object" }',
        auto_fix: "false",
      }),
    ).toEqual({ schema: { type: "object" }, auto_fix: false });

    // Unset auto_fix (the "" default) is omitted so the engine's own default
    // (true) applies; an empty schema is omitted (identity parse).
    expect(configFromDraft("output_parser", { schema: "", auto_fix: "" })).toBeUndefined();
    expect(configFromDraft("output_parser", { schema: "", auto_fix: "true" })).toEqual({
      auto_fix: true,
    });
  });

  it("sub_workflow: workflow_id", () => {
    expect(configFromDraft("sub_workflow", { workflow_id: "child-1" })).toEqual({
      workflow_id: "child-1",
    });
  });

  it("an all-empty draft serializes to undefined (config omitted from the body)", () => {
    expect(configFromDraft("switch", { field: "", expression: "" })).toBeUndefined();
    expect(configFromDraft("http_request", { method: "", url: "" })).toBeUndefined();
  });
});

describe("hydrate → serialize round-trips", () => {
  const cases: Array<{ kind: string; config: Record<string, unknown> }> = [
    { kind: "tool_call", config: { slug: "gmail.send", args: { to: "=item.email" } } },
    {
      kind: "http_request",
      config: {
        method: "POST",
        url: "=item.url",
        headers: { "X-Api-Key": "abc" },
        body: { q: "hi" },
      },
    },
    { kind: "switch", config: { field: "kind" } },
    { kind: "output_parser", config: { schema: { type: "object" }, auto_fix: false } },
    { kind: "sub_workflow", config: { workflow_id: "child-1" } },
  ];

  for (const { kind, config } of cases) {
    it(`${kind} survives a hydrate/serialize cycle`, () => {
      const { draft, extra } = configDraftFrom(kind, config);
      expect(configFromDraft(kind, draft, extra)).toEqual(config);
    });
  }
});

describe("unknown-key preservation — the data-loss guard", () => {
  it("keeps an orchestrator's connection_ref on a tool_call across an edit", () => {
    const config = { slug: "gmail.send", connection_ref: "acct_42", args: { to: "x" } };
    const { draft, extra } = configDraftFrom("tool_call", config);
    // The unknown key lands in `extra`, not in a form field.
    expect(extra).toEqual({ connection_ref: "acct_42" });
    expect(draft.slug).toBe("gmail.send");
    // …and rides back out verbatim on serialize.
    expect(configFromDraft("tool_call", draft, extra)).toEqual(config);
  });

  it("keeps a sub_workflow's execution/concurrency/inputs", () => {
    const config = {
      workflow_id: "child-1",
      execution: "per_item",
      concurrency: 3,
      inputs: { repo: "=inputs.repo" },
    };
    const { draft, extra } = configDraftFrom("sub_workflow", config);
    expect(extra).toEqual({
      execution: "per_item",
      concurrency: 3,
      inputs: { repo: "=inputs.repo" },
    });
    expect(configFromDraft("sub_workflow", draft, extra)).toEqual(config);
  });
});

describe("output never contains a host-reserved key", () => {
  it("drops a reserved key smuggled into a saved config, on both hydrate and serialize", () => {
    const config = {
      slug: "gmail.send",
      on_error: "route",
      retry: { maxAttempts: 3 },
      requires_approval: true,
      schedule: "0 9 * * *",
      destination: { kind: "owner" },
      agent_ref: "someone",
    };
    const { draft, extra } = configDraftFrom("tool_call", config);
    for (const key of RESERVED_CONFIG_KEYS) {
      expect(extra, key).not.toHaveProperty(key);
    }
    const out = configFromDraft("tool_call", draft, extra) ?? {};
    for (const key of RESERVED_CONFIG_KEYS) {
      expect(out, key).not.toHaveProperty(key);
    }
    expect(out).toEqual({ slug: "gmail.send" });
  });

  it("never re-emits a reserved key even if forced through the extra bag", () => {
    const out = configFromDraft("http_request", { method: "GET", url: "u" }, {
      on_error: "route",
      requires_approval: true,
    });
    expect(out).toEqual({ method: "GET", url: "u" });
  });
});

describe("configFieldProblem — blur-time JSON check", () => {
  const argsSpec = configFieldSpecs("tool_call").find((s) => s.key === "args")!;
  const slugSpec = configFieldSpecs("tool_call").find((s) => s.key === "slug")!;

  it("flags malformed JSON on a json field", () => {
    expect(configFieldProblem(argsSpec, "{ not json")).toMatch(/must be valid JSON/);
  });

  it("accepts valid JSON, and never nags an empty field", () => {
    expect(configFieldProblem(argsSpec, '{ "a": 1 }')).toBeNull();
    expect(configFieldProblem(argsSpec, "")).toBeNull();
    // A non-JSON control has no blur rule of its own.
    expect(configFieldProblem(slugSpec, "anything")).toBeNull();
  });

  it("rejects a valid-JSON value whose shape the engine key can't take", () => {
    const headersSpec = configFieldSpecs("http_request").find((s) => s.key === "headers")!;
    const bodySpec = configFieldSpecs("http_request").find((s) => s.key === "body")!;

    // `args`/`headers` must be objects — arrays and scalars are valid JSON but wrong.
    for (const bad of ["[]", "42", "true", '"a string"', "null"]) {
      expect(configFieldProblem(argsSpec, bad)).toMatch(/must be a JSON object/);
      expect(configFieldProblem(headersSpec, bad)).toMatch(/must be a JSON object/);
    }

    // A body may be an object OR a bare string, but not an array/number/boolean.
    expect(configFieldProblem(bodySpec, '"raw text"')).toBeNull();
    expect(configFieldProblem(bodySpec, '{ "hello": "world" }')).toBeNull();
    for (const bad of ["[]", "42", "true", "null"]) {
      expect(configFieldProblem(bodySpec, bad)).toMatch(
        /must be a JSON object or a JSON string/,
      );
    }
  });
});

describe("configDraftProblem — submit-time gate", () => {
  it("requires a tool_call slug", () => {
    expect(configDraftProblem("tool_call", "n1", { slug: "", args: "" })).toMatch(
      /needs a tool slug/,
    );
    expect(configDraftProblem("tool_call", "n1", { slug: "web_search", args: "" })).toBeNull();
  });

  it("requires an http_request url", () => {
    expect(configDraftProblem("http_request", "n1", { method: "GET", url: "" })).toMatch(
      /needs a URL/,
    );
  });

  it("rejects malformed JSON at submit even if blur was skipped", () => {
    expect(
      configDraftProblem("tool_call", "n1", { slug: "x", args: "{ oops" }),
    ).toMatch(/must be valid JSON/);
  });

  it("requires a switch to name a field or an expression", () => {
    expect(configDraftProblem("switch", "n1", { field: "", expression: "" })).toMatch(
      /field or an expression/,
    );
    expect(configDraftProblem("switch", "n1", { field: "status", expression: "" })).toBeNull();
    expect(
      configDraftProblem("switch", "n1", { field: "", expression: "=item.x" }),
    ).toBeNull();
  });

  it("rejects a switch that supplies both a field and an expression", () => {
    expect(
      configDraftProblem("switch", "n1", { field: "status", expression: "=item.x" }),
    ).toMatch(/field or an expression, not both/);
  });

  it("rejects an http_request json field whose shape is wrong at submit", () => {
    expect(
      configDraftProblem("http_request", "n1", { method: "GET", url: "u", headers: "[]" }),
    ).toMatch(/must be a JSON object/);
    expect(
      configDraftProblem("http_request", "n1", { method: "GET", url: "u", body: "42" }),
    ).toMatch(/must be a JSON object or a JSON string/);
  });

  it("requires an output_parser schema to parse when present", () => {
    expect(
      configDraftProblem("output_parser", "n1", { schema: "{ bad", auto_fix: "" }),
    ).toMatch(/must be valid JSON/);
    // Absent schema is fine — the engine treats it as identity.
    expect(configDraftProblem("output_parser", "n1", { schema: "", auto_fix: "" })).toBeNull();
  });

  it("rejects a sub_workflow pointed at its own id", () => {
    expect(
      configDraftProblem("sub_workflow", "flow-1", { workflow_id: "flow-1" }),
    ).toMatch(/can't call itself/);
    expect(
      configDraftProblem("sub_workflow", "flow-1", { workflow_id: "flow-2" }),
    ).toBeNull();
    // Missing id is a required-field problem, not the self-reference one.
    expect(configDraftProblem("sub_workflow", "flow-1", { workflow_id: "" })).toMatch(
      /needs the id/,
    );
  });

  it("is a no-op for a kind without a config form", () => {
    expect(configDraftProblem("agent", "n1", {})).toBeNull();
  });
});
