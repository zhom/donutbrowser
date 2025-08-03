import { spawn } from "node:child_process";
import path from "node:path";
import type { LaunchOptions } from "camoufox-js/dist/utils.js";
import {
  type CamoufoxConfig,
  deleteCamoufoxConfig,
  generateCamoufoxId,
  getCamoufoxConfig,
  listCamoufoxConfigs,
  saveCamoufoxConfig,
} from "./camoufox-storage.js";

/**
 * Start a Camoufox instance in a separate process
 * @param options Camoufox launch options
 * @param profilePath Profile directory path
 * @param url Optional URL to open
 * @returns Promise resolving to the Camoufox configuration
 */
export async function startCamoufoxProcess(
  options: LaunchOptions = {},
  profilePath?: string,
  url?: string,
): Promise<CamoufoxConfig> {
  // Generate a unique ID for this instance
  const id = generateCamoufoxId();

  // Create the Camoufox configuration
  const config: CamoufoxConfig = {
    id,
    options,
    profilePath,
    url,
  };

  // Save the configuration before starting the process
  saveCamoufoxConfig(config);

  // Build the command arguments
  const args = [
    path.join(__dirname, "index.js"),
    "camoufox-worker",
    "start",
    "--id",
    id,
  ];

  // Spawn the process with proper detachment - similar to proxy implementation
  const child = spawn(process.execPath, args, {
    detached: true,
    stdio: ["ignore", "pipe", "pipe"], // Capture stdout and stderr for startup feedback
    cwd: process.cwd(),
    env: {
      ...process.env,
      NODE_ENV: "production",
      // Ensure Camoufox can find its dependencies
      NODE_PATH: process.env.NODE_PATH || "",
    },
  });

  // Wait for the worker to start successfully or fail - with shorter timeout for quick response
  return new Promise<CamoufoxConfig>((resolve, reject) => {
    let resolved = false;
    let stdoutBuffer = "";
    let stderrBuffer = "";

    // Shorter timeout for quick startup feedback
    const timeout = setTimeout(() => {
      if (!resolved) {
        resolved = true;
        child.kill("SIGKILL");
        reject(
          new Error(`Camoufox worker ${id} startup timeout after 5 seconds`),
        );
      }
    }, 5000);

    // Handle stdout - look for success JSON
    if (child.stdout) {
      child.stdout.on("data", (data) => {
        const output = data.toString();
        stdoutBuffer += output;

        // Look for success JSON message
        const lines = stdoutBuffer.split("\n");
        for (const line of lines) {
          if (line.trim()) {
            try {
              const parsed = JSON.parse(line.trim());
              if (parsed.success && parsed.id === id && parsed.processId) {
                if (!resolved) {
                  resolved = true;
                  clearTimeout(timeout);
                  config.processId = parsed.processId;
                  saveCamoufoxConfig(config);

                  // Unref immediately after success to detach properly
                  child.unref();
                  resolve(config);
                  return;
                }
              }
            } catch {
              // Not JSON, continue
            }
          }
        }
      });
    }

    // Handle stderr - look for error JSON
    if (child.stderr) {
      child.stderr.on("data", (data) => {
        const output = data.toString();
        stderrBuffer += output;

        // Look for error JSON message
        const lines = stderrBuffer.split("\n");
        for (const line of lines) {
          if (line.trim()) {
            try {
              const parsed = JSON.parse(line.trim());
              if (parsed.error && parsed.id === id) {
                if (!resolved) {
                  resolved = true;
                  clearTimeout(timeout);
                  reject(
                    new Error(
                      `Camoufox worker failed: ${parsed.message || parsed.error}`,
                    ),
                  );
                  return;
                }
              }
            } catch {
              // Not JSON, continue
            }
          }
        }
      });
    }

    child.on("exit", (code, signal) => {
      if (!resolved) {
        resolved = true;
        clearTimeout(timeout);
        if (code !== 0) {
          reject(
            new Error(
              `Camoufox worker ${id} exited with code ${code} and signal ${signal}. Stderr: ${stderrBuffer}`,
            ),
          );
        } else {
          // Process exited successfully but we didn't get success message
          reject(
            new Error(
              `Camoufox worker ${id} exited without success confirmation`,
            ),
          );
        }
      }
    });
  });
}

/**
 * Stop a Camoufox process
 * @param id The Camoufox ID to stop
 * @returns Promise resolving to true if stopped, false if not found
 */
export async function stopCamoufoxProcess(id: string): Promise<boolean> {
  const config = getCamoufoxConfig(id);

  if (!config) {
    return false;
  }

  try {
    console.log(`Stopping Camoufox process ${id} (PID: ${config.processId})`);

    // Method 1: If we have a process ID, kill by PID with proper signal sequence
    if (config.processId) {
      try {
        // First try SIGTERM for graceful shutdown
        process.kill(config.processId, "SIGTERM");
        console.log(`Sent SIGTERM to Camoufox process ${config.processId}`);

        // Give it more time to terminate gracefully (increased from 2s to 5s)
        await new Promise((resolve) => setTimeout(resolve, 5000));

        // Check if process is still running
        try {
          process.kill(config.processId, 0); // Signal 0 checks if process exists
          // Process still exists, force kill
          console.log(
            `Camoufox process ${config.processId} still running, sending SIGKILL`,
          );
          process.kill(config.processId, "SIGKILL");
        } catch {
          // Process already terminated
          console.log(
            `Camoufox process ${config.processId} terminated gracefully`,
          );
        }
      } catch {
        console.log(
          `Camoufox process ${config.processId} not found or already terminated`,
        );
      }
    }

    // Method 2: Pattern-based kill as fallback
    const killByPattern = spawn(
      "pkill",
      ["-TERM", "-f", `camoufox-worker.*${id}`],
      {
        stdio: "ignore",
      },
    );

    // Wait for pattern-based kill command to complete
    await new Promise<void>((resolve) => {
      killByPattern.on("exit", () => resolve());
      // Timeout after 3 seconds
      setTimeout(() => resolve(), 3000);
    });

    // Final cleanup with SIGKILL if needed
    setTimeout(() => {
      spawn("pkill", ["-KILL", "-f", `camoufox-worker.*${id}`], {
        stdio: "ignore",
      });
    }, 1000);

    // Delete the configuration
    deleteCamoufoxConfig(id);
    return true;
  } catch {
    // Delete the configuration even if stopping failed
    deleteCamoufoxConfig(id);
    return false;
  }
}

/**
 * Stop all Camoufox processes
 * @returns Promise resolving when all instances are stopped
 */
export async function stopAllCamoufoxProcesses(): Promise<void> {
  const configs = listCamoufoxConfigs();

  const stopPromises = configs.map((config) => stopCamoufoxProcess(config.id));
  await Promise.all(stopPromises);
}
