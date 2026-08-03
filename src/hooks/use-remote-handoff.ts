import { useCallback, useEffect, useState } from "react";
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

  const refresh = useCallback(async () => {
    try {
      setStates(await getRemoteHandoffStates());
    } catch (error) {
      // Not signed in, or the app is still starting. The backend gate still
      // applies; the button is simply not pre-disabled.
      console.warn("Could not read remote handoff state:", error);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const unlisten = onRemoteHandoffChanged(setStates);
    return () => {
      void unlisten.then((off) => {
        off();
      });
    };
  }, [refresh]);

  const handoffFor = useCallback(
    (profileId: string): RemoteHandoffState | null => states[profileId] ?? null,
    [states],
  );

  return { handoffStates: states, handoffFor, refreshHandoff: refresh };
}
