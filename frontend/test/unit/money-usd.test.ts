import { describe, expect, it } from "vitest";

import { usd, usdMagnitude } from "@/lib/money";

/**
 * The boundaries that separate an honest figure from a misleading one: the
 * amounts that a whole-dollar formatter reports as `$0` or `$1` while the rows
 * beneath it say sixteen cents or fifty.
 */
describe("usd", () => {
  it("states nothing as nothing, at cent precision", () => {
    expect(usd(0)).toBe("$0.00");
    expect(usd(-0)).toBe("$0.00");
  });

  it("bounds an amount too small to state rather than calling it zero", () => {
    expect(usd(0.004)).toBe("<$0.01");
    expect(usd(0.0001)).toBe("<$0.01");
    expect(usd(-0.004)).toBe(">-$0.01");
  });

  it("keeps sub-dollar spend at the precision it was recorded in", () => {
    expect(usd(0.16)).toBe("$0.16");
    expect(usd(0.5)).toBe("$0.50");
    expect(usd(0.999)).toBe("$1.00");
    expect(usd(0.005)).toBe("$0.01");
  });

  it("decides the zero boundary the same way on either side of zero", () => {
    expect(usd(-0.005)).toBe("-$0.01");
    expect(usd(0.005)).toBe("$0.01");
    expect(usd(-0.004)).toBe(">-$0.01");
    expect(usd(-0.0049)).toBe(">-$0.01");
    expect(usd(-0)).toBe("$0.00");
  });

  it("groups a large amount without dropping its cents", () => {
    expect(usd(12345.678)).toBe("$12,345.68");
    expect(usd(1000000)).toBe("$1,000,000.00");
  });

  it("carries a sign on a real debit", () => {
    expect(usd(-0.16)).toBe("-$0.16");
    expect(usd(-1234.5)).toBe("-$1,234.50");
  });

  it("refuses to render a number it does not have", () => {
    expect(usd(Number.NaN)).toBe("—");
    expect(usd(Number.POSITIVE_INFINITY)).toBe("—");
  });
});

describe("usdMagnitude", () => {
  it("drops the sign for a caller that renders its own", () => {
    expect(usdMagnitude(-0.16)).toBe("$0.16");
    expect(usdMagnitude(0.16)).toBe("$0.16");
    expect(usdMagnitude(-0)).toBe("$0.00");
  });

  it("keeps the sub-cent bound", () => {
    expect(usdMagnitude(-0.004)).toBe("<$0.01");
  });
});

/**
 * The property that makes a tile and the list under it agree: one amount, one
 * rendering, whichever side of the screen asks for it.
 */
describe("agreement", () => {
  it("renders one amount identically for every caller", () => {
    for (const amount of [0, 0.004, 0.16, 0.5, 0.999, 42, 12345.678]) {
      expect(usd(amount)).toBe(usd(amount));
      expect(usdMagnitude(amount)).toBe(usd(Math.abs(amount)));
    }
  });

  it("never renders a positive amount as free", () => {
    for (const amount of [0.0001, 0.004, 0.0049, 0.005, 0.16]) {
      expect(usd(amount)).not.toBe("$0.00");
      expect(usd(amount)).not.toBe("$0");
    }
  });

  it("never disagrees with usdMagnitude about the same magnitude", () => {
    const magnitude = (rendered: string): string =>
      rendered.startsWith(">-") ? `<${rendered.slice(2)}` : rendered.startsWith("-") ? rendered.slice(1) : rendered;

    for (const amount of [-0.005, 0.005, -0.004, -0.0049, -0, 0.16, 0.5, 42, 12345.678]) {
      expect(magnitude(usd(amount))).toBe(usdMagnitude(amount));
    }
  });
});
