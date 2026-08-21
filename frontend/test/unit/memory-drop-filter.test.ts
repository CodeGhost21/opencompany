// What a dropped folder does NOT put into memory.
//
// A folder drop is the feature's whole point and also its whole risk: dropping
// a project directory is a natural thing to try, and without this filter it
// means uploading `.git/objects` and `node_modules` one refusal at a time —
// thousands of requests, a drop report nobody can read, and the real documents
// buried in it.
import { describe, expect, it } from "vitest";

import { ignored } from "@/views/memory/DropZone";

describe("ignored", () => {
  it("skips the noise every real folder carries", () => {
    expect(ignored(".DS_Store")).toBe(true);
    expect(ignored("Contracts/.DS_Store")).toBe(true);
    expect(ignored("Thumbs.db")).toBe(true);
  });

  it("skips version-control and dependency trees at any depth", () => {
    expect(ignored("repo/.git/objects/ab/cdef")).toBe(true);
    expect(ignored("node_modules/react/index.js")).toBe(true);
    expect(ignored("app/node_modules/react/index.js")).toBe(true);
    expect(ignored("service/target/debug/build.rs")).toBe(true);
    expect(ignored("site/.next/cache/x")).toBe(true);
  });

  it("keeps the documents an operator dropped the folder for", () => {
    expect(ignored("Contracts/2026/acme.pdf")).toBe(false);
    expect(ignored("handbook.md")).toBe(false);
    expect(ignored("finance/Q3.xlsx")).toBe(false);
  });

  it("does not skip a file that merely mentions an ignored name", () => {
    // `.gitignore` is a real file an operator may want remembered, and
    // `notes-about-node_modules.md` is prose about one.
    expect(ignored(".gitignore")).toBe(false);
    expect(ignored("notes-about-node_modules.md")).toBe(false);
    expect(ignored("docs/targeting.md")).toBe(false);
  });
});
