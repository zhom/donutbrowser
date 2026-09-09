import { useCallback, useEffect, useRef, useState } from "react";
import {
  getRemoteHandoffStates,
  onRemoteHandoffChanged,
  type RemoteHandoffState,
} from "@/lib/remote-sessions";

/**
 * Which profiles cannot be opened on this computer right now.
 *
 * Reads the same store the backend launch gate reads, so the button this
 * disables and the refusal the backend would produce can never disagree. That
 * matters more than it sounds: the previous signal was the profile-lock cache,
 * which refreshes on a 30-second server poll and only refetches on this
 * device's own lock events. A profile running on the fleet therefore looked
 * launchable for up to half a minute, and a profile whose finished session had
 * not been pulled back looked launchable indefinitely.
 *
 * Updates arrive as an event rather than a poll because every transition that
 * can change this already emits one.
 */
export function useRemoteHandoff() {
  const [states, setStates] = useState<Record<string, RemoteHandoffState>>({});
  const [ready, setReady] = useState(false);
  const revision = useRef(0);
  const mounted = useRef(false);

  const refresh = useCallback(async () => {
    const requestedRevision = ++revision.current;
    try {
      const snapshot = await getRemoteHandoffStates();
      // An older command response must never replace a newer handoff event.
      if (mounted.current && revision.current === requestedRevision) {
        setStates(snapshot);
        setReady(true);
      }
    } catch (error) {
      // Not signed in, or the app is still starting. The backend gate still
      // applies; the button is simply not pre-disabled.
      console.warn("Could not read remote handoff state:", error);
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void onRemoteHandoffChanged((snapshot) => {
      if (cancelled) return;
      ++revision.current;
      setStates(snapshot);
      setReady(true);
    })
      .then((off) => {
        if (cancelled) {
          off();
          return;
        }
        unlisten = off;
        void refresh();
      })
      .catch((error) => {
        console.warn("Could not listen for remote handoffs:", error);
        if (!cancelled) void refresh();
      });
    return () => {
      cancelled = true;
      mounted.current = false;
      ++revision.current;
      unlisten?.();
    };
  }, [refresh]);

  const handoffFor = useCallback(
    (profileId: string): RemoteHandoffState | null => states[profileId] ?? null,
    [states],
  );

  return {
    handoffStates: states,
    handoffFor,
    handoffStatesReady: ready,
    refreshHandoff: refresh,
  };
}
