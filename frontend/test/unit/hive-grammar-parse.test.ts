import { describe, expect, it } from "vitest";

import {
  GATEABLE_KINDS,
  MOVE_KINDS,
  UNGATED_KINDS,
  looksDemoted,
  markerLines,
  moveOf,
  parseMove,
} from "@/lib/hive/grammar";

/**
 * The marker grammar.
 *
 * Every case here is a way the console could invent a move the room never made,
 * or drop one it did. Both are silent: a fabricated `!propose` puts an option on
 * a floor nobody offered, and a dropped `!support` makes a carried topic look
 * unbacked. Neither throws, and neither is visible without reading the
 * transcript against the render.
 *
 * Mirrors `src/hivemind/moves.rs` and tinyhivemind's `trace::resolve`.
 */

describe("where a marker is recognised", () => {
  it("reads a marker at the start of a line", () => {
    expect(parseMove("!propose #stage ship it")?.kind).toBe("propose");
  });

  it("tolerates leading whitespace, which a model reliably adds", () => {
    expect(parseMove("   !support #stage ^4 agreed")?.kind).toBe("support");
  });

  it("refuses a marker that is not line-leading", () => {
    // The whole line is prose that mentions a move. Counting it would let any
    // paragraph discussing the grammar cast a vote.
    expect(parseMove("I would !propose #stage here")).toBeNull();
  });

  it("does not read a marker inside a fenced code block", () => {
    // The failure this prevents: a teammate pasting a sample of the grammar
    // deposits real traces, and the room carries a topic from documentation.
    const body = ["Here is how it works:", "```", "!propose #fake", "```", "done"].join(
      "\n",
    );
    expect(markerLines(body).map((l) => l.line)).not.toContain("!propose #fake");
    expect(moveOf(body)).toBeNull();
  });

  it("keeps a tilde fence and a backtick fence apart", () => {
    // A ``` block containing a ~~~ line is still one block. Closing on the wrong
    // character would expose the rest of the sample to the parser.
    const body = ["```", "~~~", "!propose #fake", "```", "!support #real ^1"].join("\n");
    expect(moveOf(body)?.topic).toBe("real");
  });

  it("does not close a long fence on a shorter example or trailing prose", () => {
    const body = [
      "````",
      "```",
      "!propose #fake",
      "```` still illustrating",
      "!support #also-fake ^1",
      "````",
      "!support #real ^1",
    ].join("\n");
    expect(moveOf(body)?.topic).toBe("real");
  });

  it("finds the move below prose when a turn opened with a greeting", () => {
    const body = "Sure — here is my read.\n!evidence #stage ^2 the last rollout broke checkout";
    expect(moveOf(body)?.kind).toBe("evidence");
  });
});

describe("arguments", () => {
  it("reads the topic, the target and every citation in order", () => {
    const move = parseMove("!object >7 ^3 ^5 that figure is stale");
    expect(move?.kind).toBe("object");
    expect(move?.target).toBe(7);
    expect(move?.cites).toEqual([3, 5]);
  });

  it("keeps the prose after the arguments are struck out", () => {
    expect(parseMove("!support #stage ^4 agreed, staging first")?.body).toBe(
      "agreed, staging first",
    );
  });

  it("treats !unpin as asking for the pin permission", () => {
    // Both write the same board, and a member entitled to put something on it may
    // take it off again. Splitting them would be a permission nobody could use.
    expect(parseMove("!unpin ^4")?.kind).toBe("pin");
  });

  it("is not a move when the word is not a kind", () => {
    expect(parseMove("!shrug #stage")).toBeNull();
  });
});

describe("markers whose arguments are mandatory", () => {
  it("discards a proposal that names no topic", () => {
    // `!propose canary` names nothing, so there is no option to put on the floor.
    expect(parseMove("!propose canary looks fine")).toBeNull();
  });

  it("discards a refutation missing its topic or its citation", () => {
    // A refutation caps a topic for the whole room, so it is grounded by
    // construction or it is nothing.
    expect(parseMove("!refute #stage no good")).toBeNull();
    expect(parseMove("!refute ^4 no good")).toBeNull();
    expect(parseMove("!refute #stage ^4 the benchmark says otherwise")?.kind).toBe(
      "refute",
    );
  });

  it("discards a deferral that abstains from nothing in particular", () => {
    expect(parseMove("!defer no view")).toBeNull();
    expect(parseMove("!defer #stage no view")?.kind).toBe("defer");
  });

  it("keeps a support with no citation, so the renderer can mark it ungrounded", () => {
    // The host folds it as depositing no supporter. The console still needs the
    // move, because "supported without grounds" is exactly the thing worth
    // showing — a transcript that reads as though it counted is the failure.
    const move = parseMove("!support #stage sounds right");
    expect(move?.kind).toBe("support");
    expect(move?.cites).toEqual([]);
  });
});

describe("demotion", () => {
  it("recognises a line whose leading ! was stripped", () => {
    // How the host records a move a seat may not make: the line still says what
    // its author meant and deposits no trace.
    expect(looksDemoted("propose #stage ship it")).toBe("propose");
  });

  it("does not call an ordinary sentence a demotion", () => {
    expect(looksDemoted("proposing that we ship it")).toBeNull();
    expect(looksDemoted("The answer is ready for someone to commit.")).toBeNull();
  });

  it("does not call a real move a demotion", () => {
    expect(looksDemoted("!propose #stage ship it")).toBeNull();
  });
});

describe("the closed set", () => {
  it("gates six kinds and never the other three", () => {
    expect(UNGATED_KINDS).toEqual(["question", "defer", "commit"]);
    expect(GATEABLE_KINDS).toEqual([
      "propose",
      "support",
      "object",
      "refute",
      "evidence",
      "pin",
    ]);
    expect(GATEABLE_KINDS.length + UNGATED_KINDS.length).toBe(MOVE_KINDS.length);
  });
});
