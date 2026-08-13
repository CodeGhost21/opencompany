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
      "`ghost` is not a wired channel — this deployment has: operator, email.",
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
