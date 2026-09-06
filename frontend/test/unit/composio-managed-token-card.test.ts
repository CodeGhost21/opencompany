import { describe, expect, it } from "vitest";

import { showManagedTokenCard } from "@/views/connections/ComposioSection";

/**
 * `showManagedTokenCard` gates the legacy managed-route credential card, and
 * both its inputs — the SELECTED tile and the PERSISTED route — have to agree
 * that the company is on managed before it renders. Either alone puts a
 * second credential surface on screen at exactly the moment an operator is
 * mid-switch between the two routes; see the function's own doc for the two
 * distinct ways that goes wrong.
 */
describe("showManagedTokenCard", () => {
  const base = {
    mode: "managed" as const,
    onByok: false,
    canManage: true,
    credentialed: false,
    showOverride: false,
    byoToken: false,
  };

  it("shows the card in the steady managed state with nothing credentialled", () => {
    expect(showManagedTokenCard(base)).toBe(true);
  });

  // The regression this whole function exists to fix: a BYOK company
  // (`onByok: true`) clicking the managed tile (`mode: "managed"`) must NOT
  // show this card. It used to — gated on the selected tile alone — and its
  // own Clear button calls the legacy `setComposioToken("")`, which erases
  // the preserved backend-token override without touching `composio/api_key`
  // or `composio/mode` at all: a control that looks like the way back to
  // managed but silently destroys a different token while leaving the
  // company on BYOK regardless.
  it("hides the card when the company is persisted BYOK, even with the managed tile selected", () => {
    expect(
      showManagedTokenCard({ ...base, mode: "managed", onByok: true, byoToken: true }),
    ).toBe(false);
  });

  // The other direction: a managed company (`onByok: false`) that has
  // selected the BYOK tile (`mode: "byok"`) must not see this card either —
  // it would sit alongside the Composio API key field with no way to tell
  // which Save it belongs to.
  it("hides the card when the BYOK tile is selected, even for a persisted-managed company", () => {
    expect(showManagedTokenCard({ ...base, mode: "byok", onByok: false })).toBe(false);
  });

  it("hides the card once both the tile and the persisted route agree on BYOK", () => {
    expect(
      showManagedTokenCard({ ...base, mode: "byok", onByok: true, byoToken: true }),
    ).toBe(false);
  });

  it("hides the card from a viewer who cannot manage the connection", () => {
    expect(showManagedTokenCard({ ...base, canManage: false })).toBe(false);
  });

  it("shows the card when an already-credentialled operator asks for the override", () => {
    expect(
      showManagedTokenCard({ ...base, credentialed: true, showOverride: false }),
    ).toBe(false);
    expect(
      showManagedTokenCard({ ...base, credentialed: true, showOverride: true }),
    ).toBe(true);
  });

  it("shows the card for a company that already pasted a backend token", () => {
    expect(showManagedTokenCard({ ...base, byoToken: true })).toBe(true);
  });
});
