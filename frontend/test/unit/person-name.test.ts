// Naming somebody who has not named themselves.
//
// The rule is mirrored in `src/ports/users.rs` (`derive_display_name`), which is
// what the host uses when it has to name a person in mail. The vectors below are
// the same ones that file's tests use, so a change to one that is not made to
// the other shows up as one person being called two things on two surfaces.

import { describe, expect, it } from "vitest";

import { guessName, personName, personAvatar } from "@/lib/person";
import { hashedFlavour } from "@/lib/avatar";

describe("guessName", () => {
  it("reads a name out of the local part", () => {
    expect(guessName("steven.enamakel@acme.com")).toBe("Steven Enamakel");
    expect(guessName("steven_enamakel@acme.com")).toBe("Steven Enamakel");
    expect(guessName("steven-enamakel@acme.com")).toBe("Steven Enamakel");
    // A routing tag is plumbing, not a middle name.
    expect(guessName("steven+board@acme.com")).toBe("Steven");
    expect(guessName("stevent95@acme.com")).toBe("Stevent95");
    // Already-capitalised local parts are left as written: lower-casing the
    // rest would turn McDonald into Mcdonald.
    expect(guessName("McDonald@acme.com")).toBe("McDonald");
    // The domain is dropped — it names the mailbox, not the person.
    expect(guessName("ada@a.very.long.domain.example")).toBe("Ada");
  });

  it("refuses to guess where there is no name", () => {
    // "Cannot say" has to stay distinguishable from a guess: a base58 key
    // title-cased would *look* like a name.
    expect(guessName("wallet:7cVfgArCheMR6Cs29HGxwPFXhAxrJ6UP3TcTZqSKz8bE")).toBeNull();
    expect(guessName("local:owner")).toBeNull();
    expect(guessName("123.456@acme.com")).toBeNull();
    expect(guessName("@acme.com")).toBeNull();
  });
});

describe("personName", () => {
  const ada = { id: "u1", email: "ada.lovelace@acme.com" };

  it("prefers what they chose, then the guess, then the key itself", () => {
    expect(personName({ ...ada, displayName: "Ada L." })).toBe("Ada L.");
    // A blank name is not a name.
    expect(personName({ ...ada, displayName: "   " })).toBe("Ada Lovelace");
    expect(personName(ada)).toBe("Ada Lovelace");
    // The last resort, and the only place a raw key is rendered: a
    // wallet-signed-in person has no name to guess, and showing their key beats
    // showing a blank where a name belongs.
    const wallet = { id: "u2", email: "wallet:7cVfgArCheMR6Cs29HGxw" };
    expect(personName(wallet)).toBe(wallet.email);
  });
});

describe("personAvatar", () => {
  it("prefers a chosen face and falls back to the mascot hashed from the id", () => {
    const ada = { id: "u1", email: "ada@acme.com" };
    expect(personAvatar({ ...ada, avatar: "blob:01J8Z5" })).toBe("blob:01J8Z5");
    expect(personAvatar(ada)).toBe(`tiny:${hashedFlavour("u1")}`);
  });
});
