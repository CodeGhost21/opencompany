import { describe, expect, it } from "vitest";

import { destinationTargetProblem } from "@/views/WorkflowCreateDialog";

// #813 defect 4: a channel destination pointing at a channel this deployment
// never wired only failed at delivery (`ChannelNotWired`). The picker keeps a
// create-mode author on the wired list, but the branch is still reachable — an
// edit dialog can carry a persisted target for a channel since unwired, and a
// free-text value can be entered during the async `listWiredChannels` load
// before the list arrives. These pin the author-time refusal on that path.
describe("destinationTargetProblem — unwired channel", () => {
  const wired = ["operator", "email"];

  it("rejects a channel that is not in the wired set, naming what is", () => {
    const problem = destinationTargetProblem("channel", "ghost", wired);
    expect(problem).toBe(
      "`ghost` is not a workflow delivery channel — this runtime has: operator, email.",
    );
  });

  it("accepts a channel that is wired", () => {
    expect(destinationTargetProblem("channel", "operator", wired)).toBeNull();
    expect(destinationTargetProblem("channel", "email", wired)).toBeNull();
  });

  it("does not reject free text when the host offered no wired list", () => {
    // Degraded/loading state: an empty list means "we don't know", so a
    // free-text box must not be wrongly refused.
    expect(destinationTargetProblem("channel", "anything", [])).toBeNull();
  });
});

/**
 * The load-order trap (issue #1053).
 *
 * `wiredChannels.length > 0` spelled two different states the same way: "the
 * host offered no channels" and "the read has not answered yet". Skipping on
 * both meant an operator quick enough to press Create before the list landed got
 * *weaker* validation than a slow one — and met the host's 400 instead of this
 * check.
 */
describe("destinationTargetProblem while the channel list is still loading", () => {
  it("defers rather than passing a channel it cannot check yet", () => {
    const problem = destinationTargetProblem("channel", "ghost", [], false);
    expect(problem).toMatch(/still checking/i);
  });

  /**
   * The distinction the flag exists for: an *answered* empty list is the
   * degraded free-text case and must stay permissive, so a host that offers no
   * channels never wrongly rejects an author.
   */
  it("stays permissive once an empty list is actually an answer", () => {
    expect(destinationTargetProblem("channel", "anything", [], true)).toBeNull();
  });

  /** Not-yet-loaded only defers a check it would otherwise make. */
  it("does not defer a destination kind it never checks against the list", () => {
    expect(destinationTargetProblem("email", "someone@example.com", [], false)).toBeNull();
  });
});
