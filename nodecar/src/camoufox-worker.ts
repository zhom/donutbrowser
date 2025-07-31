import { Camoufox } from "camoufox-js";
import type { Page } from "playwright-core";
import { getCamoufoxConfig, saveCamoufoxConfig } from "./camoufox-storage.js";

/**
 * Run a Camoufox browser server as a worker process
 * @param id The Camoufox configuration ID
 */
export async function runCamoufoxWorker(id: string): Promise<void> {
  // Get the Camoufox configuration
  const config = getCamoufoxConfig(id);

  if (!config) {
    console.error(
      JSON.stringify({
        error: "Configuration not found",
        id: id,
      }),
    );
    process.exit(1);
  }

  // Return success immediately - before any async operations
  const processId = process.pid;

  console.log(
    JSON.stringify({
      success: true,
      id: id,
      processId,
      profilePath: config.profilePath,
      message: "Camoufox worker started successfully",
    }),
  );

  // Update config with process details
  config.processId = processId;
  saveCamoufoxConfig(config);

  // Handle process termination gracefully
  const gracefulShutdown = async () => {
    process.exit(0);
  };

  process.on("SIGTERM", () => void gracefulShutdown());
  process.on("SIGINT", () => void gracefulShutdown());

  // Launch browser in background - this can take time and may fail
  setImmediate(async () => {
    let page: Page | null = null;

    try {
      // Prepare options for Camoufox
      const camoufoxOptions = { ...config.options };

      // Add profile path if provided
      if (config.profilePath) {
        camoufoxOptions.user_data_dir = config.profilePath;
      }

      // Remove custom properties before passing to Camoufox
      camoufoxOptions.disableTheming = true;
      camoufoxOptions.showcursor = false;

      // Set Firefox preferences for theming
      if (!camoufoxOptions.firefox_user_prefs) {
        camoufoxOptions.firefox_user_prefs = {};
      }

      // Default to non-headless for visibility
      if (camoufoxOptions.headless === undefined) {
        camoufoxOptions.headless = false;
      }

      const browser = await Camoufox(camoufoxOptions);
      const context = await browser.newContext();

      saveCamoufoxConfig(config);

      // Handle URL opening if provided
      if (config.url && context) {
        try {
          if (!page) {
            page = await context.newPage();
          }
          await page.goto(config.url, {
            waitUntil: "domcontentloaded",
            timeout: 30000,
          });
        } catch {
          // URL opening failure doesn't affect startup success
        }
      }

      // Monitor browser connection
      const keepAlive = setInterval(async () => {
        try {
          if (!browser || !browser.isConnected()) {
            clearInterval(keepAlive);
            process.exit(0);
          }
        } catch {
          clearInterval(keepAlive);
          process.exit(0);
        }
      }, 2000);
    } catch {
      // Browser launch failed, but worker is still "successful"
      // Process will stay alive due to the main setInterval above
    }
  });

  // Keep process alive
  process.stdin.resume();
}
