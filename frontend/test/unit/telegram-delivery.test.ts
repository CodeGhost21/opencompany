import { describe, expect, it } from "vitest";

import { telegramDelivery } from "@/lib/channels";

/**
 * The one reading the Telegram badge and the "can't collect or reply" notice
 * both consult (issue #1467).
 *
 * The bug this pins: the green "configured" badge fired on a stored token alone,
 * while the rebuild notice fired on `configured && !polling` — both satisfiable
 * at once, so a working-looking badge sat above a notice telling the operator to
 * rebuild their host. And because the status is an unvalidated cast, an older
 * host that never sent `polling` made `!polling` truthy and produced that notice
 * on the strength of a field that was never sent.
 */
describe("telegramDelivery", () => {
  it("delivers only when a token is stored AND the host reports polling", () => {
    expect(telegramDelivery({ configured: true, polling: true })).toBe("delivering");
  });

  it("reports stored-not-delivering only on an explicit polling:false", () => {
    // This is the state the rebuild notice is for — and the only one it may fire
    // on, so it can never co-occur with the badge.
    expect(telegramDelivery({ configured: true, polling: false })).toBe("stored-not-delivering");
  });

  it("treats a missing polling field as unknown, not as not-delivering", () => {
    // An older host predating the field, arriving through the unvalidated cast.
    // Neither a green badge nor a "rebuild your host" instruction is warranted.
    expect(telegramDelivery({ configured: true })).toBe("unknown");
    const drift = { configured: true, polling: undefined } as {
      configured: boolean;
      polling?: boolean;
    };
    expect(telegramDelivery(drift)).toBe("unknown");
  });

  it("is unconfigured with no token, and on a missing status", () => {
    expect(telegramDelivery({ configured: false, polling: true })).toBe("unconfigured");
    expect(telegramDelivery(null)).toBe("unconfigured");
    expect(telegramDelivery(undefined)).toBe("unconfigured");
  });

  it("gives the badge and the notice mutually exclusive triggers", () => {
    // The structural guarantee the badge/notice split relies on: no single
    // status is ever both "delivering" (badge) and "stored-not-delivering"
    // (notice), so the two can never render together.
    const verdicts = [true, false, undefined].map((polling) =>
      telegramDelivery({ configured: true, polling }),
    );
    expect(verdicts.filter((v) => v === "delivering")).toHaveLength(1);
    expect(verdicts.filter((v) => v === "stored-not-delivering")).toHaveLength(1);
  });
});
