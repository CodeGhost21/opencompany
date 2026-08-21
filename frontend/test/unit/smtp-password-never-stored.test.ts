// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";

import { scopedKey } from "@/connections/types";
import {
  emptyMailSettings,
  loadMailSettings,
  purgeStoredSmtpPasswords,
  saveMailSettings,
  withoutSmtpPassword,
} from "@/lib/domain";

/**
 * The SMTP password must never reach `localStorage` (issue #1460).
 *
 * The pre-fix console held the whole card in one `useState` and wrote all of it
 * back on every change, so a live sending credential was persisted to browser
 * storage character by character as it was typed — readable by any script on
 * the origin, surviving sign-out, with no expiry.
 *
 * The assertions below are deliberately written against **the whole store**
 * rather than against a known key. A test that checks `oc-mail:…` for a missing
 * `password` field passes the day someone adds a second key, or renames the
 * prefix, or stores a draft somewhere new — and the credential leaks again with
 * a green suite. So: type a password, do the things the card does, then read
 * every value in `localStorage` and assert the secret appears in none of them.
 * That is the property the fix exists to hold, and it cannot rot into a weaker
 * one without the assertion visibly changing.
 */

const SCOPE = { connection: "conn-a", company: "acme" };
const OTHER_SCOPE = { connection: "conn-b", company: "globex" };
const LEGACY_KEY = "oc-mail:acme";

/** Distinctive enough that a substring search over the store is meaningful. */
const SECRET = "pw-3f9a1c-do-not-persist";

beforeEach(() => {
  localStorage.clear();
});

/** Every value currently in `localStorage`, concatenated. */
function entireStore(): string {
  const parts: string[] = [];
  for (let i = 0; i < localStorage.length; i++) {
    const key = localStorage.key(i);
    if (key === null) continue;
    parts.push(key, localStorage.getItem(key) ?? "");
  }
  return parts.join("\n");
}

/** A full settings object with a password typed into it. */
function settingsWithPassword() {
  const settings = emptyMailSettings();
  settings.domain = { domain: "mail.acme.com", verified: false };
  settings.smtp = {
    host: "smtp.postmarkapp.com",
    port: "587",
    security: "starttls",
    username: "apikey",
    password: SECRET,
    fromName: "Acme",
    fromEmail: "hello@mail.acme.com",
  };
  return settings;
}

describe("saving the card", () => {
  it("puts no part of the password anywhere in localStorage", () => {
    saveMailSettings(SCOPE, withoutSmtpPassword(settingsWithPassword()));

    expect(entireStore()).not.toContain(SECRET);
  });

  it("still remembers the non-secret fields", () => {
    saveMailSettings(SCOPE, withoutSmtpPassword(settingsWithPassword()));

    const raw = localStorage.getItem(scopedKey("oc-mail", SCOPE)) ?? "";
    expect(raw).toContain("smtp.postmarkapp.com");
    expect(raw).toContain("apikey");
    expect(raw).toContain("mail.acme.com");
    // The key itself is absent, not merely empty: `withoutSmtpPassword`
    // destructures it away rather than blanking it.
    expect(raw).not.toContain("password");
  });

  it("survives the per-keystroke write pattern the card actually uses", () => {
    // The regression was a `useEffect` firing once per character. Replaying it
    // is what proves no intermediate write leaks a prefix of the secret.
    const settings = settingsWithPassword();
    for (let i = 0; i <= SECRET.length; i++) {
      saveMailSettings(SCOPE, withoutSmtpPassword({
        ...settings,
        smtp: { ...settings.smtp, password: SECRET.slice(0, i) },
      }));
    }

    expect(entireStore()).not.toContain(SECRET.slice(0, 8));
  });
});

describe("the persisted shape", () => {
  it("will not accept a full settings object", () => {
    // The load-bearing guard, and the one that cannot rot: if `saveMailSettings`
    // ever becomes willing to take a password again, this `@ts-expect-error`
    // stops being an error and `npm run typecheck:unit` fails on the unused
    // directive. A reviewer has to delete this line on purpose to reintroduce
    // the bug.
    // @ts-expect-error a full MailSettings carries `password` and must not be storable
    saveMailSettings(SCOPE, settingsWithPassword());

    // Belt and braces: even having forced the call through, the value written
    // is whatever the caller passed, so this is the assertion that would catch
    // a runtime regression the type system was talked out of.
    expect(entireStore()).toContain(SECRET);
    purgeStoredSmtpPasswords();
    expect(entireStore()).not.toContain(SECRET);
  });
});

describe("reading the card back", () => {
  it("returns an empty password even when storage still holds one", () => {
    // A key written by an older build, in a tab that has not reloaded since the
    // purge ran. It must not flow back into the form.
    localStorage.setItem(
      scopedKey("oc-mail", SCOPE),
      JSON.stringify({ ...settingsWithPassword() }),
    );

    expect(loadMailSettings(SCOPE).smtp.password).toBe("");
  });

  it("keeps the non-secret fields it reads back", () => {
    saveMailSettings(SCOPE, withoutSmtpPassword(settingsWithPassword()));

    const loaded = loadMailSettings(SCOPE);
    expect(loaded.smtp.host).toBe("smtp.postmarkapp.com");
    expect(loaded.smtp.username).toBe("apikey");
    expect(loaded.domain.domain).toBe("mail.acme.com");
  });
});

describe("purging what the old console already stored", () => {
  it("clears passwords from every scope, not just the one on screen", () => {
    const stored = JSON.stringify(settingsWithPassword());
    localStorage.setItem(scopedKey("oc-mail", SCOPE), stored);
    localStorage.setItem(scopedKey("oc-mail", OTHER_SCOPE), stored);
    localStorage.setItem(LEGACY_KEY, stored);

    expect(purgeStoredSmtpPasswords()).toBe(3);
    expect(entireStore()).not.toContain(SECRET);
  });

  it("keeps the operator's non-secret work", () => {
    localStorage.setItem(scopedKey("oc-mail", SCOPE), JSON.stringify(settingsWithPassword()));

    purgeStoredSmtpPasswords();

    const loaded = loadMailSettings(SCOPE);
    expect(loaded.smtp.host).toBe("smtp.postmarkapp.com");
    expect(loaded.smtp.username).toBe("apikey");
    expect(loaded.smtp.fromEmail).toBe("hello@mail.acme.com");
    expect(loaded.domain.domain).toBe("mail.acme.com");
  });

  it("removes a key whose JSON cannot be parsed but mentions a password", () => {
    // Cannot be shown to be clean, so it cannot be left in place.
    localStorage.setItem(scopedKey("oc-mail", SCOPE), `{"smtp":{"password":"${SECRET}"`);

    expect(purgeStoredSmtpPasswords()).toBe(1);
    expect(localStorage.getItem(scopedKey("oc-mail", SCOPE))).toBeNull();
    expect(entireStore()).not.toContain(SECRET);
  });

  it("leaves unrelated keys alone and reports nothing to clean", () => {
    localStorage.setItem("oc-tour:conn-a::acme", '{"step":3}');
    localStorage.setItem(scopedKey("oc-mail", SCOPE), JSON.stringify(withoutSmtpPassword(emptyMailSettings())));

    expect(purgeStoredSmtpPasswords()).toBe(0);
    expect(localStorage.getItem("oc-tour:conn-a::acme")).toBe('{"step":3}');
  });

  it("is idempotent", () => {
    localStorage.setItem(scopedKey("oc-mail", SCOPE), JSON.stringify(settingsWithPassword()));

    expect(purgeStoredSmtpPasswords()).toBe(1);
    expect(purgeStoredSmtpPasswords()).toBe(0);
    expect(entireStore()).not.toContain(SECRET);
  });
});
