import { describe, expect, it } from "vitest";

import { guessName, personName } from "@/lib/person";

/**
 * `guessName` mirrors `derive_display_name` in `src/ports/users.rs` — the host
 * names a person through the same rule, and the two copies are kept in step by
 * pinning the same cases here that the Rust test pins there.
 *
 * The sharp edge (issue from review): an identity key is *parsed* before a name
 * is guessed, and the parse is not "starts with `wallet:`/`local:`". Only a
 * base58 string that decodes to a 32-byte Ed25519 key after `wallet:`, and the
 * exact value `local:owner`, have no name in them. An address like
 * `wallet:ada@example.com` is an email whose local part happens to start with
 * the prefix, and it must render as a label on both sides — not as the full raw
 * address in the console and a label on the host.
 */

/** A base58-encoded 32-byte key — the same fixture `src/ports/users.rs` uses. */
const WALLET_KEY = "7cVfgArCheMR6Cs29HGxwPFXhAxrJ6UP3TcTZqSKz8bE";

describe("guessName mirrors the host's identity parse", () => {
  it("derives a name from an email local part", () => {
    expect(guessName("steven.enamakel@acme.com")).toBe("Steven Enamakel");
    expect(guessName("ada@example.com")).toBe("Ada");
    expect(guessName("McDonald@acme.com")).toBe("McDonald");
    // The domain is dropped — it names the mailbox, not the person.
    expect(guessName("ada@a.very.long.domain.example")).toBe("Ada");
  });

  it("refuses to guess a real wallet key or the local owner", () => {
    expect(guessName(`wallet:${WALLET_KEY}`)).toBeNull();
    expect(guessName("local:owner")).toBeNull();
    expect(guessName("123.456@acme.com")).toBeNull();
    expect(guessName("@acme.com")).toBeNull();
  });

  it("treats a prefixed *email* as an email, exactly as the host does", () => {
    // `wallet:ada@example.com` fails the wallet-key check (the `@` is not
    // base58), so it parses as an email whose local part is `wallet:ada` — the
    // same label the host's `derive_display_name` produces for it.
    expect(guessName("wallet:ada@example.com")).toBe("Wallet:ada");
    expect(guessName("local:owner@example.com")).toBe("Local:owner");
    // A base58-alphabet local part that is too short to be a key is an email
    // too, matching `decode_wallet_address`'s 32-byte requirement.
    expect(guessName("wallet:hi")).toBe("Wallet:hi");
  });

  it("personName falls back to the raw key only when there is truly nothing to guess", () => {
    const wallet = { id: "w1", email: `wallet:${WALLET_KEY}` };
    expect(personName(wallet)).toBe(wallet.email);
    const prefixedEmail = { id: "p1", email: "wallet:ada@example.com" };
    expect(personName(prefixedEmail)).toBe("Wallet:ada");
  });
});
