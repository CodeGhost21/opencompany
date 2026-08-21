import { useEffect, useState } from "react";
import {
  ArrowRight,
  Building2,
  Loader2,
  MailCheck,
  Monitor,
  TriangleAlert,
  Wallet,
} from "lucide-react";

import {
  fetchAuthConfig,
  fetchHubProviders,
  loginWithPassword,
  requestCode,
  requestWalletChallenge,
  verifyCode,
  verifyWalletSignature,
  type AuthConfig,
  type HubProvider,
  type SignIn,
} from "@/api/auth";
import { connectWallet, hasWallet, NoWalletError, signMessage } from "@/lib/wallet";
import type { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button, buttonVariants } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ThemeToggle } from "@/components/theme-toggle";
import { cn } from "@/lib/utils";

/** Where the ecosystem's terms live. Same documents OpenHuman links from its welcome screen. */
const TERMS_OF_USE_URL = "https://tinyhumans.gitbook.io/openhuman/legal/terms-of-use";
const PRIVACY_POLICY_URL = "https://tinyhumans.gitbook.io/openhuman/legal/privacy-policy";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /** The company's display name, when the host would tell us before sign-in. */
  companyName?: string;
  /**
   * Why they landed here, when it was not simply "not signed in yet".
   *
   * Set after a refused ecosystem sign-in *or* a magic link that would not
   * redeem — the expired-or-spent link being by far the commonest of the two,
   * since every email-mode sign-in is one and they last fifteen minutes.
   * Without it a rejected or ineligible sign-in renders an ordinary form and
   * looks like the click did nothing — the one failure mode most likely to be
   * reported as "the button is broken" (issue #1305). It never names an
   * address, so it cannot become the membership oracle the rest of this view
   * refuses to be.
   *
   * A notice is also what puts the caret in the email field below, so that
   * "request a new one" is a keystroke rather than a hunt.
   */
  notice?: string;
  /**
   * An address to start the form with, when the caller knows one.
   *
   * Only the desktop's embedded host does: a packaged install admits one
   * standing local operator and mails nothing, so a blank field asked a person
   * to guess an address they have never seen, and every guess came back as the
   * same silent acknowledgement (#632).
   *
   * A suggestion, not a lock — it seeds the field and the person can replace
   * it, which matters as soon as they invite anyone else to their own host.
   */
  suggestedEmail?: string;
  /**
   * Reports a completed sign-in.
   *
   * Handed the whole {@link SignIn}, not just the user, because a cross-origin
   * sign-in returns a session the caller has to store — this view is not where
   * a credential belongs, but it is the only place that sees one arrive.
   */
  onSignedIn: (result: SignIn) => void;
}

type Mode = "link" | "password";

/**
 * What the host says before anything has been fetched.
 *
 * `email` because that is what every company did before the mode was
 * configurable, so an older host — which has no `/auth/config` route — renders
 * exactly the screen it always did.
 */
const ASSUMED_CONFIG: AuthConfig = { mode: "email", passwords: true };

/**
 * The sign-in view: magic link by default, password for anyone who set one.
 *
 * Two rules this view must not break:
 *
 * 1. **Never say whether an account exists.** The backend answers identically
 *    for a member and a stranger, deliberately, so that nobody can enumerate a
 *    company's membership. Rendering "no such user" here would hand back the
 *    oracle the API refuses to be.
 * 2. **Never store the session.** It arrives as an HttpOnly cookie the browser
 *    keeps; there is nothing to put in localStorage and nothing for an XSS to
 *    steal.
 */
export function Login({
  client,
  company,
  companyName,
  notice,
  suggestedEmail,
  onSignedIn,
}: Props) {
  const [mode, setMode] = useState<Mode>("link");
  /**
   * The ecosystem sign-in buttons this host offers, or `[]` if it offers none.
   *
   * Asked of the host rather than assumed, because only the host knows the
   * hub's base URL and the origin the hub must return to. A self-hosted host
   * answers with an empty list and this view renders the magic-link form alone,
   * which is why a failure to fetch is swallowed: no buttons is a valid state,
   * not an error worth showing anyone.
   */
  const [hubProviders, setHubProviders] = useState<HubProvider[]>([]);
  /**
   * How this company signs people in.
   *
   * Asked of the host, never inferred from which routes fail: a wallet company
   * and a misconfigured email company both refuse `auth/request`, and only one
   * of them should be offered a wallet button.
   */
  const [authConfig, setAuthConfig] = useState<AuthConfig>(ASSUMED_CONFIG);
  const [email, setEmail] = useState(suggestedEmail ?? "");
  // The suggestion usually arrives *after* this mounts. A relaunch restores the
  // embedded connection from its remembered profile immediately, while the
  // address and the operator come over IPC a moment later — so on every launch
  // but the first, seeding the initial state alone would leave the field blank.
  //
  // Only into an empty field: a suggestion must never overwrite what somebody
  // has typed, and clearing the prefilled address on purpose (to sign in as
  // someone else) must not be undone — this runs when the *suggestion* changes,
  // which it does once.
  useEffect(() => {
    if (suggestedEmail) setEmail((current) => (current === "" ? suggestedEmail : current));
  }, [suggestedEmail]);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sent, setSent] = useState(false);
  // Only ever set on a host with no mail transport (local dev).
  const [devCode, setDevCode] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetchAuthConfig(client, company)
      .then((config) => {
        if (!cancelled) setAuthConfig(config);
      })
      .catch(() => {
        // `fetchAuthConfig` already falls back to email; this is belt and braces.
      });
    return () => {
      cancelled = true;
    };
  }, [client, company]);

  // The real config can arrive *after* someone has already switched into
  // password mode off the optimistic `ASSUMED_CONFIG`. If it turns out this
  // host does not offer passwords, password mode must not survive that —
  // otherwise the form below stays open on a route that will refuse it.
  useEffect(() => {
    if (!authConfig.passwords && mode === "password") setMode("link");
  }, [authConfig.passwords, mode]);

  useEffect(() => {
    let cancelled = false;
    fetchHubProviders(client, company)
      .then((providers) => {
        if (!cancelled) setHubProviders(providers);
      })
      .catch(() => {
        // No buttons. The form below still works, and a host that cannot answer
        // this question could not have completed the flow anyway.
      });
    return () => {
      cancelled = true;
    };
  }, [client, company]);

  async function sendLink(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const result = await requestCode(client, company, email);
      // Always the same acknowledgement, whoever they are.
      setSent(true);
      setDevCode(result.dev_code ?? null);
    } catch (err) {
      setError(friendly(err));
    } finally {
      setBusy(false);
    }
  }

  async function signInWithPassword(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      onSignedIn(await loginWithPassword(client, company, email, password));
    } catch (err) {
      setError(friendly(err));
    } finally {
      setBusy(false);
    }
  }

  async function redeemDevCode() {
    if (!devCode) return;
    setBusy(true);
    setError(null);
    try {
      onSignedIn(await verifyCode(client, company, devCode));
    } catch (err) {
      setError(friendly(err));
    } finally {
      setBusy(false);
    }
  }

  /**
   * Connect, sign the host's challenge, and exchange it for a session.
   *
   * The message is signed exactly as the host sent it. Nothing here inspects or
   * rebuilds it: the layout is the host's, versioned by its first line, and a
   * console that assembled its own would silently stop verifying the day the
   * host changed it.
   */
  async function signInWithWallet() {
    setBusy(true);
    setError(null);
    try {
      const address = await connectWallet();
      const challenge = await requestWalletChallenge(client, company, address);
      const signature = await signMessage(challenge.message);
      onSignedIn(await verifyWalletSignature(client, company, challenge.nonce, signature));
    } catch (err) {
      setError(friendly(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="min-h-svh bg-background">
      <header className="flex items-center justify-between border-b px-6 py-4">
        <div className="flex items-center gap-2">
          <div className="flex size-7 items-center justify-center rounded-md bg-primary text-primary-foreground">
            <Building2 className="size-4" />
          </div>
          <span className="text-sm font-semibold">OpenCompany</span>
        </div>
        <ThemeToggle />
      </header>

      <main className="mx-auto flex w-full max-w-md flex-col justify-center px-6 py-16">
        {/*
          The refusal, above the heading, where the eye lands first.

          Given the icon and full-strength text on purpose. `Alert`'s default
          variant is card-coloured with a muted description, which on this page
          sits on a card-coloured form — so an unadorned notice reads as chrome
          and gets skimmed past, which is barely better than the nothing it
          replaced (#1305). Not `destructive`: an expired link is the ordinary
          fate of one left in a mailbox overnight, not an error someone made.
        */}
        {notice && (
          <Alert className="mb-6" data-testid="login-notice">
            <TriangleAlert className="size-4" />
            <AlertDescription className="text-foreground">{notice}</AlertDescription>
          </Alert>
        )}

        <div className="mb-6 space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight">
            {authConfig.mode === "none" ? "" : "Sign in"}
            {companyName ? (authConfig.mode === "none" ? companyName : ` to ${companyName}`) : ""}
          </h1>
          <p className="text-sm text-muted-foreground">{subtitle(authConfig, mode)}</p>
        </div>

        {/*
          A company with no sign-in. Reaching this screen at all means the
          console could not authenticate — on a `none`-mode host every request
          is already the owner's, so the ordinary path never renders this view.
          What is left is a genuine misconfiguration: something is addressing a
          desktop company from somewhere that is not the desktop. Say that,
          rather than offering a form whose every field is refused.
        */}
        {authConfig.mode === "none" ? (
          <Card className="space-y-3 p-6">
            <div className="flex items-start gap-3">
              <Monitor className="mt-0.5 size-5 shrink-0 text-primary" />
              <div className="space-y-1">
                <p className="text-sm font-medium">There is no sign-in here</p>
                <p className="text-sm text-muted-foreground">
                  This company is used from the OpenCompany app on the computer
                  it runs on. It admits one person — whoever is at that machine —
                  and has no accounts to sign in with.
                </p>
              </div>
            </div>
            {error ? (
              <Alert variant="destructive">
                <AlertDescription>{error}</AlertDescription>
              </Alert>
            ) : null}
          </Card>
        ) : null}

        {/*
          Wallet sign-in. One button, because there is nothing to type: the
          address comes from the wallet, and typing one you do not hold proves
          nothing anyway.
        */}
        {authConfig.mode === "wallet" ? (
          <Card className="space-y-4 p-6">
            {hasWallet() ? (
              <>
                <p className="text-sm text-muted-foreground">
                  Your wallet will be asked to sign a one-time message. It is a
                  signature, not a transaction — nothing is sent and nothing is
                  spent.
                </p>
                <Button className="w-full" onClick={signInWithWallet} disabled={busy}>
                  {busy ? <Loader2 className="mr-2 size-4 animate-spin" /> : null}
                  <Wallet className="mr-2 size-4" />
                  Continue with wallet
                </Button>
              </>
            ) : (
              <div className="flex items-start gap-3">
                <Wallet className="mt-0.5 size-5 shrink-0 text-muted-foreground" />
                <div className="space-y-1">
                  <p className="text-sm font-medium">No wallet found</p>
                  <p className="text-sm text-muted-foreground">
                    This company signs people in with a Solana wallet. Install one
                    in this browser, then reload.
                  </p>
                </div>
              </div>
            )}
            {error ? (
              <Alert variant="destructive">
                <AlertDescription>{error}</AlertDescription>
              </Alert>
            ) : null}
          </Card>
        ) : null}

        {/*
          Ecosystem sign-in, above the form because it is the path most people
          take: one click, no mailbox round trip. Rendered only when the host
          says it has a hub — a self-hosted console shows the form alone.

          Each button is a plain link to a host-supplied URL, not a fetch: the
          hub's OAuth start is a top-level navigation, and the browser must own
          it so the provider's own domain appears in the address bar.
        */}
        {authConfig.mode === "email" && hubProviders.length > 0 && (
          <div className="mb-6 space-y-3">
            <div className="grid gap-2">
              {hubProviders.map((provider) => (
                <a
                  key={provider.id}
                  href={provider.startUrl}
                  className={cn(buttonVariants({ variant: "outline", size: "lg" }), "w-full")}
                >
                  Continue with {provider.label}
                </a>
              ))}
            </div>

            <p className="text-center text-2xs leading-5 text-muted-foreground">
              By continuing, you agree to the{" "}
              <a
                href={TERMS_OF_USE_URL}
                target="_blank"
                rel="noreferrer"
                className="font-medium underline underline-offset-2 hover:text-foreground"
              >
                Terms
              </a>{" "}
              and{" "}
              <a
                href={PRIVACY_POLICY_URL}
                target="_blank"
                rel="noreferrer"
                className="font-medium underline underline-offset-2 hover:text-foreground"
              >
                Privacy Policy
              </a>
              .
            </p>

            <div className="flex items-center gap-3">
              <div className="h-px flex-1 bg-border" />
              <span className="text-xs text-muted-foreground">or</span>
              <div className="h-px flex-1 bg-border" />
            </div>
          </div>
        )}

        {authConfig.mode === "email" ? (
        <Card className="p-6">
          {sent && mode === "link" ? (
            <div className="space-y-4">
              <div className="flex items-start gap-3">
                <MailCheck className="mt-0.5 size-5 shrink-0 text-primary" />
                <div className="space-y-1">
                  <p className="text-sm font-medium">Check your email</p>
                  <p className="text-sm text-muted-foreground">
                    If {email} can sign in here, a link is on its way. It expires in
                    15 minutes and works once.
                  </p>
                </div>
              </div>

              {devCode ? (
                <Alert>
                  <AlertDescription className="space-y-2">
                    <p className="text-xs">
                      This host has no email configured, so the link was returned
                      instead of sent. That only happens in local development.
                    </p>
                    <Button size="sm" onClick={redeemDevCode} disabled={busy}>
                      {busy ? <Loader2 className="size-4 animate-spin" /> : null}
                      Use it now
                    </Button>
                  </AlertDescription>
                </Alert>
              ) : null}

              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setSent(false);
                  setDevCode(null);
                }}
              >
                Use a different address
              </Button>
            </div>
          ) : (
            <form
              className="space-y-4"
              onSubmit={mode === "link" ? sendLink : signInWithPassword}
            >
              <div className="space-y-2">
                <Label htmlFor="email">Email</Label>
                <Input
                  id="email"
                  type="email"
                  autoComplete="username"
                  // Only when something was refused. Whoever is reading a
                  // notice has one thing left to do — ask for another link —
                  // and this makes it a keystroke instead of a hunt for the
                  // field. A cold visit is left alone: stealing focus on an
                  // ordinary page load moves the viewport for anyone using a
                  // screen reader or a zoomed browser, for no reason.
                  autoFocus={Boolean(notice)}
                  required
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  placeholder="you@company.com"
                  aria-describedby={
                    suggestedEmail && email === suggestedEmail ? "email-hint" : undefined
                  }
                />
                {suggestedEmail && email === suggestedEmail ? (
                  // Otherwise the prefilled address reads as somebody else's,
                  // and the natural move is to clear it — which is the one
                  // address this host will not answer for.
                  <p
                    id="email-hint"
                    className="text-xs text-muted-foreground"
                    data-testid="suggested-email-hint"
                  >
                    The operator account on this computer. Nothing is mailed —
                    the link comes back here.
                  </p>
                ) : null}
              </div>

              {mode === "password" ? (
                <div className="space-y-2">
                  <Label htmlFor="password">Password</Label>
                  <Input
                    id="password"
                    type="password"
                    autoComplete="current-password"
                    required
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                  />
                </div>
              ) : null}

              {error ? (
                <Alert variant="destructive">
                  <AlertDescription>{error}</AlertDescription>
                </Alert>
              ) : null}

              <Button type="submit" className="w-full" disabled={busy}>
                {busy ? <Loader2 className="mr-2 size-4 animate-spin" /> : null}
                {mode === "link" ? "Email me a link" : "Sign in"}
                {!busy ? <ArrowRight className="ml-2 size-4" /> : null}
              </Button>
            </form>
          )}
        </Card>
        ) : null}

        {authConfig.mode === "email" && authConfig.passwords ? (
        <div className="mt-4 text-center">
          <Button
            variant="link"
            size="sm"
            onClick={() => {
              setMode(mode === "link" ? "password" : "link");
              setError(null);
              setSent(false);
            }}
          >
            {mode === "link" ? "Use a password instead" : "Email me a link instead"}
          </Button>
        </div>
        ) : null}

        {authConfig.mode === "email" && mode === "password" ? (
          <p className="mt-2 text-center text-xs text-muted-foreground">
            Forgot it? Sign in with a link, then set a new password.
          </p>
        ) : null}
      </main>
    </div>
  );
}

/** One line under the heading, saying what this company will actually ask for. */
function subtitle(config: AuthConfig, mode: Mode): string {
  if (config.mode === "none") return "";
  if (config.mode === "wallet") return "Prove you hold the wallet. Nothing is emailed.";
  return mode === "link"
    ? "We\'ll email you a link. No password needed."
    : "Use the password you set for this company.";
}

/**
 * Renders an error without inventing detail the API withheld.
 *
 * `invalid_login` is the backend's single, deliberate answer for every failure —
 * unknown address, wrong password, expired link, spent link. It stays vague
 * here for the same reason it is vague there.
 */
function friendly(err: unknown): string {
  if (err instanceof NoWalletError) {
    return "No wallet found in this browser. Install one, then reload.";
  }
  if (err instanceof ApiError) {
    if (err.code === "auth_mode") {
      // The console asked for a sign-in this company does not have. Almost
      // always a stale tab against a host whose mode changed under it.
      return `${err.message}. Reload to see the right sign-in.`;
    }
    if (err.code === "invalid_login") {
      return "That didn't work. Check the address and password, or sign in with a link.";
    }
    if (err.status === 0) {
      return "Can't reach the company host.";
    }
    return err.message;
  }
  return "Something went wrong. Try again.";
}
