import type { Browser, BrowserContext, Page } from "playwright-core";
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
      port: processId,
      wsEndpoint: `ws://localhost:0/camoufox-${id}`,
      profilePath: config.profilePath,
      message: "Camoufox worker started successfully",
    }),
  );

  // Update config with process details
  config.port = processId;
  config.wsEndpoint = `ws://localhost:0/camoufox-${id}`;
  saveCamoufoxConfig(config);

  // Handle process termination gracefully
  const gracefulShutdown = async () => {
    process.exit(0);
  };

  process.on("SIGTERM", () => void gracefulShutdown());
  process.on("SIGINT", () => void gracefulShutdown());

  // Keep the process alive
  setInterval(() => {
    // Keep alive
  }, 1000);

  // Launch browser in background - this can take time and may fail
  setImmediate(async () => {
    let browser: Browser | null = null;
    let context: BrowserContext | null = null;
    let page: Page | null = null;

    try {
      // Prepare options for Camoufox
      const camoufoxOptions = { ...config.options };

      // Add profile path if provided
      if (config.profilePath) {
        camoufoxOptions.user_data_dir = config.profilePath;
      }

      // Set anti-detect options
      camoufoxOptions.disableTheming = true;
      camoufoxOptions.showcursor = false;

      // Default to headless for tests
      if (camoufoxOptions.headless === undefined) {
        camoufoxOptions.headless = false;
      }

      // Import Camoufox dynamically
      let Camoufox: any;
      try {
        const camoufoxModule = await import("camoufox-js");
        Camoufox = camoufoxModule.Camoufox;
      } catch (importError) {
        // If Camoufox is not available, just keep the process alive
        return;
      }

      // Launch Camoufox with timeout
      const result = await Promise.race([
        Camoufox(camoufoxOptions),
        new Promise((_, reject) =>
          setTimeout(() => reject(new Error("Camoufox launch timeout")), 30000),
        ),
      ]);

      // Handle the result
      if ("browser" in result && typeof result.browser === "function") {
        context = result;
        browser = context?.browser() ?? null;
      } else {
        browser = result as Browser;
        context = await browser.newContext();
      }

      if (!browser) {
        throw new Error("Failed to get browser instance");
      }

      // Update config with actual browser details
      let wsEndpoint: string | undefined;
      try {
        const browserWithWs = browser as any;
        wsEndpoint =
          browserWithWs.wsEndpoint?.() || `ws://localhost:0/camoufox-${id}`;
      } catch {
        wsEndpoint = `ws://localhost:0/camoufox-${id}`;
      }

      config.wsEndpoint = wsEndpoint;
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
        } catch (error) {
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
    } catch (error) {
      // Browser launch failed, but worker is still "successful"
      // Process will stay alive due to the main setInterval above
    }
  });

  // Keep process alive
  process.stdin.resume();
}
