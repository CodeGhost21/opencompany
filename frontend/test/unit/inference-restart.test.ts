import { describe, expect, it } from "vitest";

import { restartInference } from "@/api/inference";
import type { OpenCompanyClient } from "@/api/client";

/**
 * The action behind the "Restart required" notice.
 *
 * Before this, the notice named a restart and offered no way to perform one —
 * a dead end for exactly the operator most likely to see it, since a hosted
 * tenant cannot restart its own container and the control plane has no button
 * for it either. These pin the client half: the route it calls, and the fact
 * that the console reads the *resulting* status rather than assuming success.
 */

/** A client stub that records the call and answers with a staged body. */
function stubClient(reply: unknown): {
  client: OpenCompanyClient;
  calls: Array<{ path: string; body: unknown }>;
} {
  const calls: Array<{ path: string; body: unknown }> = [];
  const client = {
    scopeFor: (company: string | null) =>
      company ? `/api/v1/companies/${company}` : "/api/v1/company",
    post: async (path: string, body: unknown) => {
      calls.push({ path, body });
      return reply;
    },
  } as unknown as OpenCompanyClient;
  return { client, calls };
}

describe("restarting a company from the console", () => {
  it("posts to the company's own restart route", async () => {
    const { client, calls } = stubClient({ status: { restartRequired: false }, note: "" });
    await restartInference(client, "acme");
    expect(calls[0].path).toBe("/api/v1/companies/acme/inference/restart");
  });

  it("uses the single-company alias when no id is addressed", async () => {
    // A prosumer `opencompany serve` has no company id, and hard-coding the
    // multi-company form would 404 there.
    const { client, calls } = stubClient({ status: { restartRequired: false }, note: "" });
    await restartInference(client, null);
    expect(calls[0].path).toBe("/api/v1/company/inference/restart");
  });

  it("reports the status the host came back with", async () => {
    // The console must follow this rather than assume the rebuild worked: a
    // host that wired no rebuilder genuinely cannot do it, and claiming success
    // would swap a visible dead end for an invisible one.
    const { client } = stubClient({
      status: { restartRequired: true },
      note: "restart the process",
    });
    const result = await restartInference(client, "acme");
    expect(result.status.restartRequired).toBe(true);
    expect(result.note).toContain("restart the process");
  });
});
