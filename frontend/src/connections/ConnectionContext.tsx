// Which connection the surrounding subtree belongs to.
//
// Deliberately narrow. The client keeps travelling as a prop — it is threaded
// through ~134 sites already, and that threading is a compile-time guarantee
// that a view cannot reach a host it was not handed. Converting those to a
// context would trade that guarantee for brevity, and the bug it would let in
// (a view reading the wrong connection's data) is invisible rather than loud.
//
// What context is for is the handful of leaves that need to know *which*
// connection they are in without being handed a client: browser-local storage
// keys, a status pill, error copy that names the host.

import { createContext, useContext } from "react";

import type { LocalScope } from "./types";

const ScopeContext = createContext<LocalScope | null>(null);

export function ConnectionScopeProvider({
  scope,
  children,
}: {
  scope: LocalScope;
  children: React.ReactNode;
}) {
  return <ScopeContext.Provider value={scope}>{children}</ScopeContext.Provider>;
}

/**
 * The (connection, company) this subtree is rendering.
 *
 * Throws outside a provider rather than returning a default. A default would be
 * a *shared* namespace that two connections silently write to — the exact
 * failure this scope exists to prevent — and it would look like it worked.
 */
export function useLocalScope(): LocalScope {
  const scope = useContext(ScopeContext);
  if (!scope) {
    throw new Error(
      "useLocalScope() outside a ConnectionScopeProvider: browser-local state has no " +
        "connection to belong to, and guessing one would share it between hosts",
    );
  }
  return scope;
}
