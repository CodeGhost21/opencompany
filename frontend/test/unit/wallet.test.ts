import { describe, expect, it } from "vitest";

import { base58 } from "@/lib/wallet";

/**
 * The base58 encoder sits on the critical path of wallet sign-in: the
 * signature the browser wallet produces is encoded here before the host
 * checks it, and a bug in this hand-rolled encoder fails every wallet login
 * with the same `invalid_login` a wrong signature gets — indistinguishable
 * from the host's side. Pinned against known vectors rather than only
 * exercised indirectly through a mocked `signMessage`.
 */
describe("base58", () => {
  it("encodes the empty input as the empty string", () => {
    expect(base58(new Uint8Array([]))).toBe("");
  });

  it("encodes a single zero byte as one leading '1', not two", () => {
    expect(base58(new Uint8Array([0]))).toBe("1");
  });

  it("encodes each leading zero byte as its own '1'", () => {
    expect(base58(new Uint8Array([0, 0]))).toBe("11");
    expect(base58(new Uint8Array([0, 0, 0]))).toBe("111");
  });

  it("matches the known Bitcoin base58 vector for 0x00010966776006953D5567439E5E39F86A0D273BEED61967F6", () => {
    const bytes = Uint8Array.from(
      Buffer.from("00010966776006953D5567439E5E39F86A0D273BEED61967F6", "hex"),
    );
    expect(base58(bytes)).toBe("16UwLL9Risc3QfPqBUvKofHmBQ7wMtjvM");
  });

  it("round-trips a leading zero byte alongside nonzero bytes", () => {
    // A leading zero byte must still contribute exactly one '1', with the
    // rest of the value encoded normally after it.
    expect(base58(new Uint8Array([0, 1]))).toBe(`1${base58(new Uint8Array([1]))}`);
  });

  it("never folds two different byte strings onto the same encoding", () => {
    const a = base58(Uint8Array.from({ length: 32 }, (_, i) => i));
    const b = base58(Uint8Array.from({ length: 32 }, (_, i) => (i === 0 ? 1 : i)));
    expect(a).not.toBe(b);
  });
});
