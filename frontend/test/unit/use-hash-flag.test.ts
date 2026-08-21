// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { useHashFlag } from "@/hooks/use-hash-flag";

let container: HTMLDivElement;
let root: Root;
let last: { on: boolean; set: (on: boolean) => void } | null;

function Probe({ flag }: { flag: string }) {
  const [on, set] = useHashFlag(flag);
  last = { on, set };
  return null;
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  last = null;
  window.location.hash = "";
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
  window.location.hash = "";
});

describe("useHashFlag", () => {
  it("reads false when the flag is absent from the current hash", async () => {
    window.location.hash = "#/ledgers/goals";
    await act(async () => {
      root.render(createElement(Probe, { flag: "new" }));
    });
    expect(last?.on).toBe(false);
  });

  it("reads true when the flag is already present", async () => {
    window.location.hash = "#/ledgers/goals?new";
    await act(async () => {
      root.render(createElement(Probe, { flag: "new" }));
    });
    expect(last?.on).toBe(true);
  });

  it("setting true appends the flag without disturbing the path", async () => {
    window.location.hash = "#/ledgers/goals";
    await act(async () => {
      root.render(createElement(Probe, { flag: "new" }));
    });
    await act(async () => {
      last?.set(true);
    });
    expect(window.location.hash).toBe("#/ledgers/goals?new");
    expect(last?.on).toBe(true);
  });

  it("setting false removes it, restoring the bare path", async () => {
    window.location.hash = "#/ledgers/goals?new";
    await act(async () => {
      root.render(createElement(Probe, { flag: "new" }));
    });
    await act(async () => {
      last?.set(false);
    });
    expect(window.location.hash).toBe("#/ledgers/goals");
    expect(last?.on).toBe(false);
  });

  it("a hashchange (the browser Back button) is picked up without calling set", async () => {
    window.location.hash = "#/ledgers/goals?new";
    await act(async () => {
      root.render(createElement(Probe, { flag: "new" }));
    });
    expect(last?.on).toBe(true);

    // Simulates Back popping the `?new` entry, the way `useHashFlag`'s own
    // setter pushed it — a real history entry, not `replaceState`.
    await act(async () => {
      window.location.hash = "#/ledgers/goals";
      window.dispatchEvent(new HashChangeEvent("hashchange"));
    });
    expect(last?.on).toBe(false);
  });

  it("two flags on the same hash are independent", async () => {
    window.location.hash = "#/ledgers/goals?new";
    let secondSet: ((on: boolean) => void) | undefined;
    function TwoFlags() {
      const [, setNew] = useHashFlag("new");
      const [otherOn, setOther] = useHashFlag("other");
      secondSet = setOther;
      last = { on: otherOn, set: setNew };
      return null;
    }
    await act(async () => {
      root.render(createElement(TwoFlags));
    });
    await act(async () => {
      secondSet?.(true);
    });
    expect(window.location.hash).toContain("new");
    expect(window.location.hash).toContain("other");
  });
});
