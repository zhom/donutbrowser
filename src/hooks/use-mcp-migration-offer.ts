import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import type { McpMigrationOffer } from "@/lib/mcp";

/** How often a due offer looks again for a moment with no dialog open. */
const QUIET_POLL_MS = 500;

/**
 * pending: nothing decided yet. checking: asking the backend. clear: nothing
 * to offer on this launch so far. due: waiting for no other dialog. open and
 * closed: the offer took this launch.
 */
type Phase = "pending" | "checking" | "clear" | "due" | "open" | "closed";

/**
 * Any open dialog, popover or palette. Read from the page rather than from
 * each owner's state: dialogs set their own `data-slot`, but every one of
 * them carries the role. Sub-pages are in-flow pages and carry none.
 */
function anyDialogOpen(): boolean {
  return (
    document.querySelector('[role="dialog"], [role="alertdialog"]') !== null
  );
}

/**
 * The move from the removed local MCP server to remote MCP, offered by
 * itself at most once per account (the backend remembers who had it) and at
 * most once per launch. It asks again when a different account signs in.
 *
 * Until it has decided, it holds the automatic tip back, and once it takes a
 * launch it keeps holding it: the two never open on the same launch.
 */
export function useMcpMigrationOffer({
  ready,
  accountId,
}: {
  /** The launch is settled and no automatic tip has opened on it. */
  ready: boolean;
  /** The signed-in cloud user, or null. */
  accountId: string | null;
}) {
  const [phase, setPhase] = useState<Phase>("pending");
  // Read by the effects without re-running them: the check sets `checking`
  // itself, and an effect keyed on the phase would cancel its own answer.
  const phaseRef = useRef<Phase>("pending");
  const checkedForRef = useRef<string | null>(null);

  const moveTo = useCallback((next: Phase) => {
    phaseRef.current = next;
    setPhase(next);
  }, []);

  useEffect(() => {
    if (!ready) return;
    const current = phaseRef.current;
    if (current !== "pending" && current !== "clear") return;
    if (!accountId) {
      if (current === "pending") moveTo("clear");
      return;
    }
    if (checkedForRef.current === accountId) return;
    checkedForRef.current = accountId;
    moveTo("checking");
    invoke<McpMigrationOffer>("get_mcp_migration_offer")
      .then((offer) => {
        if (phaseRef.current === "checking") {
          moveTo(offer.due ? "due" : "clear");
        }
      })
      .catch((error: unknown) => {
        console.error("Failed to check the remote MCP offer:", error);
        if (phaseRef.current === "checking") moveTo("clear");
      });
  }, [ready, accountId, moveTo]);

  // A due offer waits for the paid welcome, a tip the user opened, or any
  // other dialog to close, then opens and is remembered as offered.
  useEffect(() => {
    if (phase !== "due" || !ready) return;
    const tryOpen = () => {
      if (anyDialogOpen()) return false;
      moveTo("open");
      invoke("mark_mcp_migration_offered").catch((error: unknown) => {
        console.error("Failed to remember the remote MCP offer:", error);
      });
      return true;
    };
    if (tryOpen()) return;
    const timer = window.setInterval(() => {
      if (tryOpen()) window.clearInterval(timer);
    }, QUIET_POLL_MS);
    return () => window.clearInterval(timer);
  }, [phase, ready, moveTo]);

  const close = useCallback(() => {
    if (phaseRef.current === "open") moveTo("closed");
  }, [moveTo]);

  return {
    open: phase === "open",
    /** The automatic tip must wait, or skip this launch. */
    holdsLaunch: phase !== "clear",
    close,
  };
}
