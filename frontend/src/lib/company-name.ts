// The company display name's client-side length rule, shared by every field
// that accepts one.

/**
 * The host's own limit (`COMPANY_NAME_MAX_CHARS`,
 * `src/server/ops/company_profile.rs`). The API's rejection is the real
 * enforcement point whatever the client does; clamping here only saves a round
 * trip on a pasted document.
 */
export const COMPANY_NAME_MAX_CHARS = 200;

/**
 * Clamps `value` to {@link COMPANY_NAME_MAX_CHARS}, counting Unicode scalar
 * values the way the host's `chars().count()` does.
 *
 * Not the DOM's `maxLength`, which counts UTF-16 code units: a name of astral
 * characters (most emoji, some scripts) costs two units each, so that attribute
 * refuses input at half the length the host accepts. `Array.from` iterates by
 * code point, which is the host's definition exactly.
 */
export function clampToCompanyNameLimit(value: string): string {
  const points = Array.from(value);
  return points.length <= COMPANY_NAME_MAX_CHARS
    ? value
    : points.slice(0, COMPANY_NAME_MAX_CHARS).join("");
}
