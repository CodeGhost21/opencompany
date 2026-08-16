// Company-agnostic runtime configuration.
//
// The console works against ANY OpenCompany host and ANY company. Resolution
// order (first match wins), so the same build drops in anywhere:
//   1. URL query params: ?api=<url>&company=<id>&token=<t>
//   2. window.OPENCOMPANY_CONFIG (injected in index.html for static hosting)
//   3. Vite build-time env: VITE_OC_API / VITE_OC_COMPANY / VITE_OC_TOKEN
//   4. Defaults: same-origin API, single-company mode (no id)

export interface ConsoleConfig {
  /** Base URL of the OpenCompany host. Empty string means same-origin. */
  baseUrl: string;
  /**
   * The company id to operate. `null` selects single-company mode, which uses
   * the host's `/api/v1/company/*` aliases for the sole registered company.
   */
  company: string | null;
  /**
   * A **platform** bearer token, for the hosting layer.
   *
   * `null` for humans, which is the normal case: people sign in and the
   * session rides in an HttpOnly cookie. The operator token this once carried
   * no longer exists — there is no shared-secret path into a company.
   */
  operatorToken: string | null;
  /**
   * A ready-made `x-opencompany-session` value, when this client carries its
   * own session instead of relying on a cookie.
   *
   * Never resolved from the environment or the URL — only supplied per
   * connection by `connectionConfig`, after a sign-in that asked the host for
   * the header carrier. It lives here because it describes *how a client
   * authenticates*, which is what this type is for; anywhere else and the
   * client would be taking its credential from two places.
   */
  sessionHeader: string | null;
  /**
   * Whether this deployment is a **hub**: one console serving many hosts that
   * live on other origins, rather than a console served by the host it
   * operates.
   *
   * The difference is entirely in the bootstrap. An ordinary console assumes
   * its own origin is a host and opens a connection to it; a hub's origin
   * serves static assets and nothing else, so that assumption yields a
   * connection which can only ever fail — the browser's exact equivalent of the
   * dead same-origin row the desktop used to carry (issue #613).
   *
   * A hub holds no directory of its own. The hosts it knows are the ones
   * somebody added, remembered in `localStorage` by `profileStore`.
   */
  hub: boolean;
}

declare global {
  interface Window {
    OPENCOMPANY_CONFIG?: Partial<ConsoleConfig>;
  }
}

function fromQuery(): Partial<ConsoleConfig> {
  const q = new URLSearchParams(window.location.search);
  const out: Partial<ConsoleConfig> = {};
  const api = q.get("api");
  const company = q.get("company");
  const token = q.get("token");
  if (api !== null) out.baseUrl = api;
  if (company !== null) out.company = company;
  // `?token=` is ours ONLY when the hub did not put it there. The hub's OAuth
  // callback returns `?token=<platform jwt>&key=auth`, and that token is a
  // credential for the *hub*, not a platform token for this host. Reading it
  // here would attach someone's ecosystem bearer to every API call the console
  // makes — see `signInWithHubToken`, which hands it to the host once and lets
  // it go. `key=auth` is the hub's own marker for that redirect.
  if (token !== null && q.get("key") !== "auth") out.operatorToken = token;
  return out;
}

function fromEnv(): Partial<ConsoleConfig> {
  const env = import.meta.env;
  const out: Partial<ConsoleConfig> = {};
  if (env.VITE_OC_API) out.baseUrl = env.VITE_OC_API;
  if (env.VITE_OC_COMPANY) out.company = env.VITE_OC_COMPANY;
  if (env.VITE_OC_TOKEN) out.operatorToken = env.VITE_OC_TOKEN;
  return out;
}

/** Resolves the effective console configuration once, at startup. */
export function resolveConfig(): ConsoleConfig {
  const merged: Partial<ConsoleConfig> = {
    ...fromEnv(),
    ...(window.OPENCOMPANY_CONFIG ?? {}),
    ...fromQuery(),
  };
  // Normalize a trailing slash off the base URL.
  const baseUrl = (merged.baseUrl ?? "").replace(/\/$/, "");
  return {
    baseUrl,
    company: merged.company ?? null,
    operatorToken: merged.operatorToken ?? null,
  };
}
