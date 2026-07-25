import { attachConsole } from "@tauri-apps/plugin-log";

let consoleAttached = false;

export async function setupLogging() {
  if (consoleAttached) {
    return;
  }

  try {
    await attachConsole();
    consoleAttached = true;
  } catch (err) {
    // If attachConsole fails, log to regular console as fallback
    console.error("Failed to attach console to logging plugin:", err);
  }
}
