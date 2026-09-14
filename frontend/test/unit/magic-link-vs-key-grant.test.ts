import { describe, expect, it } from "vitest";

import { readMagicLinkFrom } from "@/App";

/**
 * A landing URL says which flow it belongs to, and `?code=` alone does not.
 *
 * The key-grant return leg comes back as `?company=…&key=link&state=…&code=…`.
 * Before this, the magic-link read took any `?code=` it saw, so a successful
 * grant round trip ended with the console posting the grant code to
 * `/auth/verify` — 409 `auth_mode` on a company with no sign-in, and a "that
 * sign-in didn't complete" notice everywhere else, over a flow that had
 * worked. The code it should have used was already captured for the card that
 * asked for it.
 */
describe("readMagicLinkFrom", () => {
  it("reads an unmarked code as a magic link", () => {
    expect(readMagicLinkFrom("?company=acme&code=abc123")).toEqual({
      company: "acme",
      code: "abc123",
    });
  });

  it("leaves a key-grant return to the grant flow", () => {
    expect(readMagicLinkFrom("?company=acme&key=link&state=s1&code=grant-code")).toBeNull();
  });

  it("leaves a hub sign-in return to the sign-in flow", () => {
    // `key=auth` carries `?token=`, but a hub that also appended a `code=` must
    // not be able to steer this into redeeming one.
    expect(readMagicLinkFrom("?key=auth&token=t1&code=whatever")).toBeNull();
  });

  it("is null with no code at all", () => {
    expect(readMagicLinkFrom("?company=acme")).toBeNull();
  });
});
