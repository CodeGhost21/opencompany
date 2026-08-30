// Whether Chat's channel rail is compact, per connection and company.
//
// This belongs in the browser rather than the host: it is an operator's
// reading-space preference, and the user record has no preferences surface.
// `scopedKey` keeps two connections serving companies with the same name from
// adopting each other's layout.

import { type LocalScope, scopedKey } from "@/connections/types";

const KEY = (scope: LocalScope): string => scopedKey("oc.chat.channel-rail", scope);
const COLLAPSED = "collapsed";

function storage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** Whether this connection and company last used Chat's compact channel rail. */
export function readChannelRailCollapsed(scope: LocalScope): boolean {
  try {
    return storage()?.getItem(KEY(scope)) === COLLAPSED;
  } catch {
    return false;
  }
}

/** Remember Chat's rail density without making storage availability a UI error. */
export function writeChannelRailCollapsed(scope: LocalScope, collapsed: boolean): void {
  try {
    const store = storage();
    if (!store) return;
    if (collapsed) store.setItem(KEY(scope), COLLAPSED);
    else store.removeItem(KEY(scope));
  } catch {
    // Private mode and quota failures leave the rail usable; they only lose memory.
  }
}
