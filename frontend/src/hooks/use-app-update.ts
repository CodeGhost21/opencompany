import { useCallback, useEffect, useRef, useState } from "react";

import {
  type AppUpdateInfo,
  checkAppUpdate,
  downloadAppUpdate,
  installAppUpdate,
} from "@/api/transport/desktop";
import { isDesktopRuntime } from "@/api/transport";
import { type AppUpdatePhase, probeIsSuperseded } from "@/lib/app-update";

/**
 * The desktop shell's update flow: probing, downloading, and the restart.
 *
 * All of the logic, so that `AppUpdatePrompt` can be a component with no
 * decisions in it. What the operator is shown is [`isActionable`] in
 * `lib/app-update.ts`; what happens is here.
 *
 * # A download nobody asked for
 *
 * The check runs on a timer and, when it finds something, the download starts
 * on its own. That is deliberate and it is what makes the prompt honest: by the
 * time anybody is asked anything, the bytes are already on disk and verified,
 * so "Restart now" takes seconds rather than opening a progress bar the
 * operator then has to sit through. The cost is bandwidth on a release day,
 * which is the right thing to spend to avoid interrupting somebody twice.
 *
 * # Phases are driven from here, not from an event bus
 *
 * Each phase is set around the `await` that causes it, rather than pushed from
 * Rust over a channel. The core reports progress to nothing because nothing
 * renders progress — the download is silent by design — so a channel would
 * carry bytes no surface displays and give the phase machine two writers. If a
 * future surface wants a progress bar (an "About this app" panel, say), that is
 * when the channel earns its place.
 *
 * # Nothing happens in a browser
 *
 * `isDesktopRuntime()` gates every timer and every call. The web console is
 * whatever the host served on the last page load; it has no bundle to replace,
 * and it must not run a fifteen-minute timer to be told so.
 */

/** Delay before the first probe, so launch is not competing with a download. */
const INITIAL_CHECK_DELAY_MS = 5_000;

/** How often to re-probe while the application stays open. */
const RECHECK_INTERVAL_MS = 15 * 60 * 1000;

export interface UseAppUpdateOptions {
  /** Probe automatically. Off in tests that drive `check` by hand. */
  autoCheck?: boolean;
  initialCheckDelayMs?: number;
  /** `0` or less disables the repeat, leaving only the one probe after launch. */
  recheckIntervalMs?: number;
  /** Start the download as soon as a check finds something. */
  autoDownload?: boolean;
}

export interface UseAppUpdate {
  phase: AppUpdatePhase;
  /** The last check's answer — versions and release notes. */
  info: AppUpdateInfo | null;
  /** Why the last failing step failed, in the core's words. */
  error: string | null;
  /** Probe now. Does not download. */
  check: () => Promise<void>;
  /** Fetch and stage the offered build. Normally automatic; exposed to retry. */
  download: () => Promise<void>;
  /** Apply the staged build and relaunch. Returns only if that failed. */
  install: () => Promise<void>;
  /** Drop the error and go back to resting, without re-probing. */
  reset: () => void;
}

export function useAppUpdate(options: UseAppUpdateOptions = {}): UseAppUpdate {
  const {
    autoCheck = true,
    initialCheckDelayMs = INITIAL_CHECK_DELAY_MS,
    recheckIntervalMs = RECHECK_INTERVAL_MS,
    autoDownload = true,
  } = options;

  const [phase, setPhase] = useState<AppUpdatePhase>("idle");
  const [info, setInfo] = useState<AppUpdateInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Read by callbacks that must not be re-created when the phase moves —
  // re-creating `check` would restart the interval effect on every probe.
  const phaseRef = useRef<AppUpdatePhase>(phase);
  phaseRef.current = phase;
  // A download already running, or bytes already staged: a re-probe must not
  // throw either away, and the auto-download effect must not fire twice for
  // one detection.
  const busyRef = useRef(false);

  const check = useCallback(async () => {
    if (!isDesktopRuntime()) return;
    if (probeIsSuperseded(phaseRef.current, busyRef.current)) return;

    setPhase("checking");
    const answer = await checkAppUpdate();
    // Asked again on the way out, and that is the whole point of asking twice:
    // this probe may have started before the lid closed and be landing after
    // the one that woke with the machine has already staged a build. Writing
    // its answer now would put `up-to-date` over `ready` and take the banner
    // off screen for good. See `probeIsSuperseded`.
    if (probeIsSuperseded(phaseRef.current, busyRef.current)) return;
    setInfo(answer);
    setPhase(answer?.available ? "available" : "up-to-date");
  }, []);

  const download = useCallback(async () => {
    if (!isDesktopRuntime() || busyRef.current) return;

    busyRef.current = true;
    setError(null);
    setPhase("downloading");
    try {
      const staged = await downloadAppUpdate();
      if (staged?.ready) {
        setPhase("ready");
        return;
      }
      // The offer disappeared between the check and the download — a release
      // pulled, or another instance got there first. Nothing to say about it.
      busyRef.current = false;
      setPhase("up-to-date");
    } catch (failure) {
      busyRef.current = false;
      setError(failure instanceof Error ? failure.message : String(failure));
      setPhase("error");
    }
  }, []);

  const install = useCallback(async () => {
    if (!isDesktopRuntime()) return;

    setError(null);
    setPhase("installing");
    try {
      await installAppUpdate();
      // Only reachable if the core returned without restarting, which it does
      // not do on success. Treated as a failure so the banner does not sit on
      // "Installing" for the rest of the session.
      busyRef.current = false;
      setError("The application did not restart. Quit and reopen it to finish.");
      setPhase("error");
    } catch (failure) {
      // The staged bytes are consumed either way, so the retry in the error
      // banner has to start from a fresh download rather than another install.
      busyRef.current = false;
      setError(failure instanceof Error ? failure.message : String(failure));
      setPhase("error");
    }
  }, []);

  const reset = useCallback(() => {
    setError(null);
    setPhase((current) => (current === "error" ? "idle" : current));
  }, []);

  // One probe shortly after launch, then a slow repeat while the window stays
  // open. Both are cleared on unmount, so a console that navigates away leaves
  // no timer behind.
  useEffect(() => {
    if (!autoCheck || !isDesktopRuntime()) return;

    const first = setTimeout(() => void check(), Math.max(0, initialCheckDelayMs));
    const repeat =
      recheckIntervalMs > 0 ? setInterval(() => void check(), recheckIntervalMs) : undefined;

    return () => {
      clearTimeout(first);
      if (repeat !== undefined) clearInterval(repeat);
    };
  }, [autoCheck, check, initialCheckDelayMs, recheckIntervalMs]);

  // A check that found something starts the download itself, so the operator is
  // only ever asked to restart — never to download.
  useEffect(() => {
    if (!autoDownload || phase !== "available") return;
    void download();
  }, [autoDownload, download, phase]);

  return { phase, info, error, check, download, install, reset };
}
