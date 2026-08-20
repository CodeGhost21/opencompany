import { describe, expect, it } from "vitest";

import { credentialAffordance } from "@/views/connections/McpServersSection";

/**
 * Issue #1260. The MCP row offered a **Sign in** button on a server whose OAuth
 * could never complete: Slack's MCP endpoint answers `401` with a proper
 * resource-metadata challenge but advertises no RFC 7591 dynamic client
 * registration, so there is no client to mint. `POST …/oauth/start` refused
 * with a `400` naming the real remedy — paste a static token — which the
 * operator could only read by pressing a button that could not work.
 *
 * The host now distinguishes the two states. These pin the console half: which
 * control a row offers is a function of that hint and nothing else.
 */
describe("credentialAffordance", () => {
  it("offers sign-in only when the host says sign-in can complete", () => {
    expect(credentialAffordance("oauth_required")).toBe("sign_in");
  });

  it("offers a token field when OAuth is required but undrivable", () => {
    expect(credentialAffordance("static_token_required")).toBe("add_token");
  });

  it("never offers sign-in for a server that cannot complete one", () => {
    // The regression itself. Both codes carry `status: needs_config` and both
    // read as "an auth problem"; only this distinction stops the unusable
    // button coming back.
    expect(credentialAffordance("static_token_required")).not.toBe("sign_in");
  });

  it("routes a plain credential prompt to the same field", () => {
    expect(credentialAffordance("credential_required")).toBe("add_token");
  });

  it("offers nothing for a healthy server or an unknown code", () => {
    // A server that probed `ok` carries no hint at all, and must not sprout a
    // credential control; an unrecognised future code must not either, because
    // guessing which control it wants is how the wrong one gets offered.
    expect(credentialAffordance(undefined)).toBe("none");
    expect(credentialAffordance("token_rejected")).toBe("none");
    expect(credentialAffordance("some_code_added_later")).toBe("none");
  });
});
