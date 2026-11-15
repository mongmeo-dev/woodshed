import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  AUTO_INSTALL,
  type AvailableUpdate,
  checkForUpdate,
  currentVersion,
  updatesSupported,
} from "../lib/updater";

/** First check after launch, once the screens have settled. */
export const STARTUP_CHECK_DELAY_MS = 3000;

export type UpdateStatus =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "upToDate" }
  | { kind: "available"; version: string }
  | { kind: "downloading"; version: string; progress: number | null }
  | { kind: "checkFailed" }
  | { kind: "installFailed"; version: string };

interface UpdaterContextValue {
  status: UpdateStatus;
  /** This app's version, or null outside the app. */
  version: string | null;
  supported: boolean;
  check: () => Promise<void>;
  /** Only on a user's click: the restart stops whatever is playing. */
  install: () => Promise<void>;
}

const UpdaterContext = createContext<UpdaterContextValue | null>(null);

interface UpdaterProviderProps {
  children: ReactNode;
  startupDelayMs?: number;
}

export function UpdaterProvider({
  children,
  startupDelayMs = STARTUP_CHECK_DELAY_MS,
}: UpdaterProviderProps) {
  const supported = updatesSupported();
  const [status, setStatus] = useState<UpdateStatus>({ kind: "idle" });
  const [version, setVersion] = useState<string | null>(null);
  const update = useRef<AvailableUpdate | null>(null);
  const busy = useRef(false);
  const mounted = useRef(true);

  const install = useCallback(async () => {
    const found = update.current;
    if (!found || busy.current) return;
    busy.current = true;
    setStatus({ kind: "downloading", version: found.version, progress: null });
    try {
      await found.install((progress) => {
        if (mounted.current) setStatus({ kind: "downloading", version: found.version, progress });
      });
    } catch {
      if (mounted.current) setStatus({ kind: "installFailed", version: found.version });
    } finally {
      busy.current = false;
    }
  }, []);

  const runCheck = useCallback(
    async (quiet: boolean) => {
      if (busy.current) return;
      busy.current = true;
      let found: AvailableUpdate | null = null;
      setStatus((previous) => (quiet ? previous : { kind: "checking" }));
      try {
        found = await checkForUpdate();
        update.current = found;
        if (mounted.current) {
          setStatus(found ? { kind: "available", version: found.version } : { kind: "upToDate" });
        }
      } catch {
        // A launch check fails quietly (offline and the like).
        if (mounted.current && !quiet) setStatus({ kind: "checkFailed" });
      } finally {
        busy.current = false;
      }
      if (found && AUTO_INSTALL) {
        // Marker for scripts/check-release-dist.mjs: never in a release bundle.
        console.info("woodshed-update-test: installing without a click");
        await install();
      }
    },
    [install],
  );

  const check = useCallback(() => runCheck(false), [runCheck]);

  useEffect(() => {
    mounted.current = true;
    if (!supported) return;
    currentVersion().then(
      (value) => mounted.current && setVersion(value),
      () => {},
    );
    const timer = window.setTimeout(() => void runCheck(true), startupDelayMs);
    return () => {
      mounted.current = false;
      window.clearTimeout(timer);
    };
  }, [supported, startupDelayMs, runCheck]);

  const value = useMemo(
    () => ({ status, version, supported, check, install }),
    [status, version, supported, check, install],
  );
  return <UpdaterContext.Provider value={value}>{children}</UpdaterContext.Provider>;
}

export function useUpdater(): UpdaterContextValue {
  const value = useContext(UpdaterContext);
  if (!value) throw new Error("useUpdater needs an UpdaterProvider");
  return value;
}

/** An update that the install button can start (again after a failure). */
export function canInstall(status: UpdateStatus): boolean {
  return status.kind === "available" || status.kind === "installFailed";
}
