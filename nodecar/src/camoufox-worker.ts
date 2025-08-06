import { launchOptions } from "donutbrowser-camoufox-js";
import type { LaunchOptions } from "donutbrowser-camoufox-js/dist/utils.js";
import { type Browser, type BrowserServer, firefox } from "playwright-core";
import { getCamoufoxConfig, saveCamoufoxConfig } from "./camoufox-storage.js";
import { getEnvVars, parseProxyString } from "./utils.js";

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

  config.processId = process.pid;
  saveCamoufoxConfig(config);

  console.log(
    JSON.stringify({
      success: true,
      id: id,
      processId: process.pid,
      profilePath: config.profilePath,
      message: "Camoufox worker started successfully",
    }),
  );

  // Launch browser in background - this can take time and may fail
  setImmediate(async () => {
    let browser: Browser | null = null;
    let server: BrowserServer | null = null;
    let windowCheckInterval: NodeJS.Timeout | null = null;

    // Graceful shutdown handler with access to browser and server
    const gracefulShutdown = async () => {
      try {
        // Clear any intervals first
        if (windowCheckInterval) {
          clearInterval(windowCheckInterval);
        }

        // Close browser context and server if they exist
        if (browser?.isConnected()) {
          await browser.close();
        }
        if (server) {
          server.process().kill();
          await server.close();
        }
      } catch {
        // Ignore cleanup errors during shutdown
      }
      process.exit(0);
    };

    // Handle various quit signals for proper macOS Command+Q support
    process.on("SIGTERM", () => void gracefulShutdown());
    process.on("SIGINT", () => void gracefulShutdown());
    process.on("SIGHUP", () => void gracefulShutdown());
    process.on("SIGQUIT", () => void gracefulShutdown());

    // Handle uncaught exceptions and unhandled rejections
    process.on("uncaughtException", () => void gracefulShutdown());
    process.on("unhandledRejection", () => void gracefulShutdown());

    try {
      // Deep clone to avoid reference sharing and ensure fresh configuration for each instance
      const camoufoxOptions: LaunchOptions = JSON.parse(
        JSON.stringify(config.options || {}),
      );

      // Add profile path if provided
      if (config.profilePath) {
        camoufoxOptions.user_data_dir = config.profilePath;
      }

      // Ensure block options are properly set
      if (camoufoxOptions.block_images) {
        camoufoxOptions.block_images = true;
      }

      if (camoufoxOptions.block_webgl) {
        camoufoxOptions.block_webgl = true;
      }

      if (camoufoxOptions.block_webrtc) {
        camoufoxOptions.block_webrtc = true;
      }

      // Check for headless mode from config (no environment variable check)
      if (camoufoxOptions.headless) {
        camoufoxOptions.headless = true;
      }

      // Always set these defaults - ensure they are applied for each instance
      camoufoxOptions.i_know_what_im_doing = true;
      camoufoxOptions.config = {
        disableTheming: true,
        showcursor: false,
        ...(camoufoxOptions.config || {}),
      };

      // Generate fresh options for this specific instance
      const generatedOptions = await launchOptions(camoufoxOptions);

      // Start with process environment to ensure proper inheritance
      let finalEnv = { ...process.env };

      // Add generated options environment variables
      if (generatedOptions.env) {
        finalEnv = { ...finalEnv, ...generatedOptions.env };
      }

      // If we have a custom config from Rust, use it directly as environment variables
      if (config.customConfig) {
        try {
          // Parse the custom config JSON string
          const customConfigObj = JSON.parse(config.customConfig);

          // Convert custom config to environment variables using getEnvVars
          const customEnvVars = getEnvVars(customConfigObj);

          // Merge custom config with generated config (custom takes precedence)
          finalEnv = { ...finalEnv, ...customEnvVars };
        } catch (error) {
          console.error(
            `Camoufox worker ${id}: Failed to parse custom config, using generated config:`,
            error,
          );
          return;
        }
      }
      // Launch the server with the final configuration - ensure unique wsPath for each instance
      const finalOptions: any = {
        ...generatedOptions,
        user_data_dir: config.profilePath,
        wsPath: `/ws_${config.id}`,
        env: finalEnv,
      };

      // Only add proxy if it exists and is valid
      if (camoufoxOptions.proxy) {
        try {
          finalOptions.proxy = parseProxyString(camoufoxOptions.proxy);
        } catch (error) {
          console.error({
            message: "Failed to parse proxy, launching without proxy",
            error,
          });
          return;
        }
      }

      server = await firefox.launchServer(finalOptions);

      // Connect to the server
      browser = await firefox.connect(server.wsEndpoint());
      const context = await browser.newContext();

      // Handle browser disconnection for proper cleanup
      browser.on("disconnected", () => void gracefulShutdown());

      saveCamoufoxConfig(config);

      // Monitor for window closure
      const startWindowMonitoring = () => {
        windowCheckInterval = setInterval(async () => {
          try {
            if (browser?.isConnected()) {
              const contexts = browser.contexts();
              let totalPages = 0;

              for (const ctx of contexts) {
                const pages = ctx.pages();
                totalPages += pages.length;
              }

              // If no pages are open, terminate the server
              if (totalPages === 0) {
                if (windowCheckInterval) {
                  clearInterval(windowCheckInterval);
                }
                await gracefulShutdown();
              }
            }
          } catch {
            // If we can't check windows, assume browser is closing
            if (windowCheckInterval) {
              clearInterval(windowCheckInterval);
            }
            await gracefulShutdown();
          }
        }, 1000); // Check every second
      };

      // Handle URL opening if provided
      if (config.url) {
        try {
          const page = await context.newPage();
          await page.goto(config.url, {
            waitUntil: "domcontentloaded",
            timeout: 30000,
          });

          // Start monitoring after page is created
          startWindowMonitoring();
        } catch (urlError) {
          console.error({
            message: "Failed to open URL",
            error: urlError,
          });
          // URL opening failure doesn't affect startup success
          // Still start monitoring
          startWindowMonitoring();
        }
      } else {
        await context.newPage();
        // Start monitoring after page is created
        startWindowMonitoring();
      }

      // Monitor browser connection
      const keepAlive = setInterval(async () => {
        try {
          if (!browser || !browser.isConnected()) {
            clearInterval(keepAlive);
            await gracefulShutdown();
          }
        } catch (error) {
          console.error({
            message: "Error in keepAlive check",
            error,
          });
          clearInterval(keepAlive);
          await gracefulShutdown();
        }
      }, 2000);
    } catch (error) {
      console.error({
        message: "Failed to launch Camoufox",
        error,
      });
      // Browser launch failed, attempt cleanup
      await gracefulShutdown();
    }
  });

  // Keep process alive
  process.stdin.resume();
}
