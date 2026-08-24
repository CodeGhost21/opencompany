// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { Me } from "@/api/auth";
import type { OpenCompanyClient } from "@/api/client";
import { ProfileRow } from "@/components/profile-row";
import { SidebarProvider } from "@/components/ui/sidebar";

/**
 * Which person the sidebar footer shows while the operator moves between
 * companies (issue #1676 review).
 *
 * `me` is a read keyed by the scope it was fetched for. The failure this pins:
 * when the company prop changes, the previous company's record used to stay on
 * screen — and, worse, was the one a save through the profile dialog would have
 * written to the new company — until the new fetch happened to resolve. A pure
 * test cannot reach that: the bug is the *gap* between the prop change and the
 * new fetch landing, which only exists once the component is mounted and
 * re-rendered.
 */

function deferred<T>() {
  let settle!: (value: T) => void;
  const promise = new Promise<T>((resolve) => {
    settle = resolve;
  });
  return { promise, settle };
}

function meFor(company: string): Me {
  return { id: `${company}-me`, email: `me@${company}.test`, displayName: `${company} user` };
}

function host() {
  const reads: string[] = [];
  const beta = deferred<Me>();
  const client = {
    scopeFor: (company: string | null) =>
      company === null ? "/api/v1/company" : `/api/v1/companies/${company}`,
    get: async (path: string) => {
      const company = path.match(/companies\/([^/]+)\//)?.[1] ?? "";
      reads.push(company);
      // Only the second company's read is held open, so the test can look at
      // the row between "the scope changed" and "the new identity arrived".
      if (company === "beta") return beta.promise;
      // A company with no sign-in has no `me` to read; the console answers 404
      // and the row is expected to stay empty.
      if (company === "ghost") throw new Error("no sign-in");
      return meFor(company);
    },
  } as unknown as OpenCompanyClient;
  return { client, reads, releaseBeta: (who: Me) => beta.settle(who) };
}

let container: HTMLDivElement;
let root: Root;

async function show(client: OpenCompanyClient, company: string | null) {
  await act(async () => {
    root.render(
      createElement(
        SidebarProvider,
        null,
        createElement(ProfileRow, { client, company }),
      ),
    );
  });
}

function text(): string {
  return container.textContent ?? "";
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the sidebar identity while the operator changes company", () => {
  it("drops the previous company's identity the moment the scope changes", async () => {
    const { client, reads, releaseBeta } = host();
    await show(client, "alpha");
    expect(text()).toContain("alpha user");
    expect(reads).toEqual(["alpha"]);

    // Move to another company and leave the new fetch in flight.
    await show(client, "beta");

    // The row must not keep showing alpha's identity while beta's fetch is
    // pending — that is the bug: the sidebar, and a save through the dialog,
    // would be operating on the previous company's person.
    expect(text()).not.toContain("alpha user");

    // The beta identity lands when the fetch resolves.
    await act(async () => {
      releaseBeta(meFor("beta"));
    });
    expect(text()).toContain("beta user");
  });

  it("shows no identity for a company with no sign-in", async () => {
    const { client, reads } = host();
    await show(client, "alpha");
    expect(text()).toContain("alpha user");

    // Moving to a company whose fetch rejects (no sign-in) must leave the row
    // empty rather than restoring the previous company's identity.
    await act(async () => {
      root.render(
        createElement(
          SidebarProvider,
          null,
          createElement(ProfileRow, { client, company: "ghost" }),
        ),
      );
    });
    expect(reads).toEqual(["alpha", "ghost"]);
    expect(text()).not.toContain("alpha user");
  });
});
