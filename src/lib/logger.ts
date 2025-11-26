import {
  attachConsole,
  debug,
  error,
  info,
  trace,
  warn,
} from "@tauri-apps/plugin-log";

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

export const logger = {
  error: (message: string, ...args: unknown[]) => {
    error(`${message} ${args.map((arg) => JSON.stringify(arg)).join(" ")}`);
  },
  warn: (message: string, ...args: unknown[]) => {
    warn(`${message} ${args.map((arg) => JSON.stringify(arg)).join(" ")}`);
  },
  info: (message: string, ...args: unknown[]) => {
    info(`${message} ${args.map((arg) => JSON.stringify(arg)).join(" ")}`);
  },
  debug: (message: string, ...args: unknown[]) => {
    debug(`${message} ${args.map((arg) => JSON.stringify(arg)).join(" ")}`);
  },
  log: (message: string, ...args: unknown[]) => {
    trace(`${message} ${args.map((arg) => JSON.stringify(arg)).join(" ")}`);
  },
};
