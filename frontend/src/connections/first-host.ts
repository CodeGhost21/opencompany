// What to say to somebody who has no host at all.
//
// The one screen where "where does my company run" is not a setting but the
// only question there is, and — before connectors — the one screen that
// answered it by pointing at a control somewhere else.
//
// ## Two situations, and they are not the same absence
//
// The desktop reaches this when the host inside it did not start: usually
// another copy of the application, or an `opencompany serve` in a terminal,
// holding the data root. Something went wrong, and the operator can fix it.
//
// A **hub** reaches it by simply being new. Its own origin serves assets and
// nothing else (`hub-console.md`), so a hub that nobody has added a host to
// yet holds zero connections and always did. Nothing went wrong, and the
// desktop's copy — "the host on this computer didn't start" — describes a
// computer that was never going to run one.
//
// The same words for both is how a first run reads as a failure.
//
// See `docs/spec/runtime/connectors.md`.

/** The lines this screen shows, and what its button offers. */
export interface FirstHostCopy {
  title: string;
  body: string;
  /**
   * The button.
   *
   * There is one in both situations, and that is the point: this used to say
   * "add a host from the switcher above", which names a control instead of
   * being one. A dead end that describes its own exit is still a dead end —
   * and on the hub it was describing the exit from a different building.
   */
  action: string;
}

/**
 * @param desktop whether this runtime can start a host itself, which is what
 * separates "it didn't come up" from "there has never been one".
 */
export function firstHostCopy(desktop: boolean): FirstHostCopy {
  if (desktop) {
    return {
      title: "No host to show",
      body:
        "The host on this computer didn't start — another copy of OpenCompany may be " +
        "holding its data. Quit the other copy and reopen this one, or run your company " +
        "somewhere else.",
      action: "Choose where to run",
    };
  }
  return {
    title: "No company connected yet",
    body:
      "Choose where your company runs: hosted for you on TinyHumans Cloud, or a gateway " +
      "you run yourself.",
    action: "Choose where to run",
  };
}
