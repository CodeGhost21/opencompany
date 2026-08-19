// @vitest-environment jsdom

import { describe, expect, it } from "vitest";

import { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import type {
  StreamHandlers,
  Transport,
  TransportRequest,
  TransportResponse,
} from "@/api/transport";

/**
 * The client boundary that turns the host's structured 400 into an `ApiError`
 * the create dialog can render per node (issue #1016).
 *
 * The backend half already ships: a `workflow_invalid` refusal carries a
 * `problems` array alongside the `{error, code}` envelope, each entry naming a
 * `node_id`/`field`/`message`. This suite pins the one place that array crosses
 * into the app — `httpError` — because everything downstream (the dialog
 * mapping a problem to a node's config field) is only as good as the parse
 * here. The wire is snake_case (`node_id`); the app is camelCase (`nodeId`),
 * and the map between them lives at exactly this seam.
 */

/** Answers every request with one staged status + body, so a single 400 can be
 * driven through the real request path. */
class StubTransport implements Transport {
  constructor(
    private readonly status: number,
    private readonly body: string,
  ) {}

  async request(req: TransportRequest): Promise<TransportResponse> {
    return {
      status: this.status,
      statusText: "",
      url: req.url,
      text: this.body,
      header: () => null,
    };
  }

  subscribe(_url: string, _handlers: StreamHandlers): () => void {
    return () => {};
  }
}

function clientReturning(status: number, body: string): OpenCompanyClient {
  return new OpenCompanyClient(
    { baseUrl: "", company: "acme", operatorToken: null, sessionHeader: null },
    new StubTransport(status, body),
  );
}

describe("the client surfacing a workflow_invalid 400", () => {
  it("carries the host's problems, mapping node_id → nodeId", async () => {
    const client = clientReturning(
      400,
      JSON.stringify({
        error: "the workflow could not be validated",
        code: "workflow_invalid",
        problems: [{ node_id: "greet", field: "config.url", message: "m" }],
      }),
    );

    const err = await client.post("/api/v1/companies/acme/workflows", {}).then(
      () => {
        throw new Error("the request should have been refused");
      },
      (e) => e as unknown,
    );

    expect(err).toBeInstanceOf(ApiError);
    const api = err as ApiError;
    expect(api.status).toBe(400);
    expect(api.code).toBe("workflow_invalid");
    expect(api.problems).toHaveLength(1);
    // The snake→camel rename is the whole point of the seam.
    expect(api.problems?.[0].nodeId).toBe("greet");
    expect(api.problems?.[0].field).toBe("config.url");
    expect(api.problems?.[0].message).toBe("m");
  });

  it("carries a problem whose node_id and field the host omitted", async () => {
    // Both are optional on the wire (a graph-level complaint names neither);
    // the parse must not invent them.
    const client = clientReturning(
      400,
      JSON.stringify({
        error: "invalid",
        code: "workflow_invalid",
        problems: [{ message: "needs exactly one trigger" }],
      }),
    );

    const api = (await client
      .put("/api/v1/companies/acme/workflows/wf", {})
      .catch((e) => e)) as ApiError;

    expect(api.problems).toHaveLength(1);
    expect(api.problems?.[0].nodeId).toBeUndefined();
    expect(api.problems?.[0].field).toBeUndefined();
    expect(api.problems?.[0].message).toBe("needs exactly one trigger");
  });

  it("drops a malformed problem entry whose message is not a string", async () => {
    const client = clientReturning(
      400,
      JSON.stringify({
        error: "invalid",
        code: "workflow_invalid",
        problems: [
          { node_id: "a", field: "config.url", message: "kept" },
          { node_id: "b", message: 42 },
        ],
      }),
    );

    const api = (await client
      .post("/api/v1/companies/acme/workflows", {})
      .catch((e) => e)) as ApiError;

    expect(api.problems).toHaveLength(1);
    expect(api.problems?.[0].nodeId).toBe("a");
  });

  it("leaves problems undefined on an ordinary envelope-only 400", async () => {
    const client = clientReturning(
      400,
      JSON.stringify({ error: "that id is taken", code: "bad_request" }),
    );

    const api = (await client
      .post("/api/v1/companies/acme/workflows", {})
      .catch((e) => e)) as ApiError;

    expect(api.code).toBe("bad_request");
    expect(api.message).toBe("that id is taken");
    expect(api.problems).toBeUndefined();
  });
});
