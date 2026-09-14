// The Room's data model, as one import.
//
// The implementation lives in three focused modules — `channels.ts` (what a
// channel is), `timeline.ts` (how its rows are grouped), `review.ts` (a card's
// lifecycle inside a conversation). This barrel exists so the ~39 specs and the
// components that grew up importing `model` keep one stable address for all of
// it, rather than every call site having to know which of the three a helper
// ended up in.
//
// Nothing is declared here. A new export belongs in whichever module owns its
// subject, and reaches this file by the re-export below.

export * from "./channels";
export * from "./review";
export * from "./timeline";
