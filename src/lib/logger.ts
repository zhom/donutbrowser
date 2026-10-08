import { attachConsole, error as logError } from "@tauri-apps/plugin-log";

let consoleAttached = false;
let errorsForwarded = false;

function describe(reason: unknown): string {
  if (reason instanceof Error) return reason.stack ?? reason.message;
  return typeof reason === "string" ? reason : JSON.stringify(reason);
}

export async function setupLogging() {
  if (consoleAttached) {
    return;
  }

  // Only uncaught failures go to the log file. Forwarding console.error too
  // would loop: attachConsole writes every backend line back to the console.
  if (!errorsForwarded) {
    errorsForwarded = true;
    window.addEventListener("error", (event) => {
      void logError(
        `Uncaught error err="${describe(event.error ?? event.message)}" source=${event.filename}:${event.lineno}:${event.colno}`,
      );
    });
    window.addEventListener("unhandledrejection", (event) => {
      void logError(`Unhandled rejection err="${describe(event.reason)}"`);
    });
  }

  try {
    await attachConsole();
    consoleAttached = true;
  } catch (err) {
    // If attachConsole fails, log to regular console as fallback
    console.error("Failed to attach console to logging plugin:", err);
  }
}
