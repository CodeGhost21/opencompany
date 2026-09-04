import { describe, expect, it } from "vitest";

import { OpenCompanyClient } from "@/api/client";
import type { BlockerVerdict } from "@/api/types";
import { blockerEventVerdict } from "@/api/types";
import type { Transport, TransportRequest, TransportResponse } from "@/api/transport";

/**
 * **P1 review finding (Codex) on PR #2038.** This client explicitly supports
 * hosts older than a given field, and `blocker_verdict` is no exception: a host
 * predating it ignores the unknown key and resolves from the lowered two-way
 * `verdict` alone.
 *
 * Two of the four survive that lowering and two do not. `retry` rides an
 * `approve` and `cancel` rides a `deny`, and an unaware host retries and
 * cancels exactly as asked. `skip` also rides an `approve`, so an unaware host
 * **re-runs the step it was asked to leave out**; `amend` rides one too, so it
 * re-runs the step **without the operator's words**. In both cases the console
 * reported the four-way result it thought it had requested while the host
 * performed a different action — the silent divergence this asserts against.
 *
 * The host now says which it is at `/spec` (`blocker-verdict`), so the fix is
 * to check before sending rather than to send and hope. An absent
 * `capabilities` array is an older host still and must read as "assume REST
 * only", never as "supports nothing in particular".
 */

/** What the canned host answered, and everything it was asked. */
interface Harness {
  client: OpenCompanyClient;
  sent: { method: string; url: string; body: Record<string, unknown> }[];
}

/**
 * A client over a host whose `/spec` answers `capabilities`. `null` is a host
 * predating the field entirely — it answers a spec with no such key.
 */
function hostAdvertising(capabilities: string[] | null): Harness {
  const sent: Harness["sent"] = [];
  const transport: Transport = {
    request: async ({ method, url, body }: TransportRequest): Promise<TransportResponse> => {
      sent.push({ method, url, body: body ? JSON.parse(body) : {} });
      const spec = url.endsWith("/spec");
      const payload = spec
        ? { instance_id: "i1", ...(capabilities ? { capabilities } : {}) }
        : { recorded: true, alreadyResolved: false };
      return {
        status: 200,
        statusText: "",
        url,
        text: JSON.stringify(payload),
        header: () => null,
      };
    },
    subscribe: () => () => {},
  };
  return {
    client: new OpenCompanyClient(
      { baseUrl: "", company: null, operatorToken: null, sessionHeader: null },
      transport,
    ),
    sent,
  };
}

/** Every resolve this host was actually asked to perform. */
function resolves(h: Harness) {
  return h.sent.filter((r) => r.method === "POST" && r.url.includes("/approvals/"));
}

const MODERN = ["rest", "graphql", "sse", "approvals", "blocker-verdict"];
const LEGACY = ["rest", "graphql", "sse", "approvals"];

/** The answer each verdict carries, matching what the host refuses without. */
function answerFor(verdict: BlockerVerdict) {
  return verdict === "amend" ? "use gpt-4o-mini instead" : undefined;
}

async function answer(h: Harness, verdict: BlockerVerdict) {
  return h.client.resolveApproval("b1", blockerEventVerdict(verdict), undefined, null, {
    blocker: { verdict, answer: answerFor(verdict) },
  });
}

describe("a four-way blocker answer is negotiated before it is sent", () => {
  it("sends all four to a host that advertises the capability", async () => {
    for (const verdict of ["retry", "amend", "skip", "cancel"] as BlockerVerdict[]) {
      const h = hostAdvertising(MODERN);
      await answer(h, verdict);
      expect(resolves(h)).toHaveLength(1);
      expect(resolves(h)[0].body.blocker_verdict).toBe(verdict);
    }
  });

  it("refuses skip and amend on a host that does not advertise it, sending nothing", async () => {
    for (const verdict of ["skip", "amend"] as BlockerVerdict[]) {
      const h = hostAdvertising(LEGACY);
      await expect(
        answer(h, verdict),
        `${verdict} lowers to an approve this host would act on differently, so it must \
be refused rather than sent`,
      ).rejects.toThrow();
      expect(
        resolves(h),
        `${verdict} must not reach a host that cannot carry it out`,
      ).toHaveLength(0);
    }
  });

  it("still sends retry and cancel to an old host, which lower faithfully", async () => {
    for (const verdict of ["retry", "cancel"] as BlockerVerdict[]) {
      const h = hostAdvertising(LEGACY);
      await answer(h, verdict);
      expect(
        resolves(h),
        `${verdict} means the same thing to an unaware host, so refusing it would drop \
a working control`,
      ).toHaveLength(1);
      expect(resolves(h)[0].body.verdict).toBe(blockerEventVerdict(verdict));
    }
  });

  it("treats a host predating the capabilities field as unable, not as able", async () => {
    const h = hostAdvertising(null);
    await expect(answer(h, "skip")).rejects.toThrow();
    expect(resolves(h)).toHaveLength(0);
  });

  it("leaves an ordinary approval alone, negotiating nothing", async () => {
    const h = hostAdvertising(LEGACY);
    await h.client.resolveApproval("a1", "approve", undefined, null, {});
    expect(resolves(h)).toHaveLength(1);
    expect(resolves(h)[0].body).toEqual({ verdict: "approve" });
  });
});
