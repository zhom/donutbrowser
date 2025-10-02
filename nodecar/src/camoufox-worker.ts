import fs from "node:fs";
import path from "node:path";
import { launchOptions } from "donutbrowser-camoufox-js";
import type { LaunchOptions } from "donutbrowser-camoufox-js/dist/utils.js";
import { type Browser, type BrowserContext, firefox } from "playwright-core";
import tmp from "tmp";
import { getCamoufoxConfig, saveCamoufoxConfig } from "./camoufox-storage.js";
import { getEnvVars, parseProxyString } from "./utils.js";

// Set up debug logging to a file
const LOG_DIR = path.join(tmp.tmpdir, "donutbrowser", "camoufox-logs");
if (!fs.existsSync(LOG_DIR)) {
  fs.mkdirSync(LOG_DIR, { recursive: true });
}

function debugLog(id: string, message: string, data?: any): void {
  const logFile = path.join(LOG_DIR, `${id}.log`);
  const timestamp = new Date().toISOString();
  const logMessage = data
    ? `[${timestamp}] ${message}: ${JSON.stringify(data, null, 2)}\n`
    : `[${timestamp}] ${message}\n`;
  fs.appendFileSync(logFile, logMessage);
}

/**
 * Run a Camoufox browser server as a worker process
 * @param id The Camoufox configuration ID
 */
export async function runCamoufoxWorker(id: string): Promise<void> {
  debugLog(id, "Worker starting", { pid: process.pid });

  // Get the Camoufox configuration
  debugLog(id, "Loading Camoufox configuration");
  const config = getCamoufoxConfig(id);

  if (!config) {
    debugLog(id, "Configuration not found");
    console.error(
      JSON.stringify({
        error: "Configuration not found",
        id: id,
      }),
    );
    process.exit(1);
  }

  debugLog(id, "Configuration loaded successfully", {
    profilePath: config.profilePath,
    hasOptions: !!config.options,
    hasCustomConfig: !!config.customConfig,
    hasUrl: !!config.url,
  });

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
    debugLog(id, "Starting browser launch in background");
    let browser: Browser | null = null;
    let context: BrowserContext | null = null;
    let windowCheckInterval: NodeJS.Timeout | null = null;

    // Graceful shutdown handler with access to browser and server
    const gracefulShutdown = async () => {
      debugLog(id, "Graceful shutdown initiated");
      try {
        // Clear any intervals first
        if (windowCheckInterval) {
          clearInterval(windowCheckInterval);
        }

        // Close browser context and server if they exist
        if (context && !context.pages) {
          // Context is already closed
        } else if (context) {
          await context.close();
        }

        if (browser?.isConnected()) {
          await browser.close();
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
      debugLog(id, "Preparing launch options");
      // Deep clone to avoid reference sharing and ensure fresh configuration for each instance
      const camoufoxOptions: LaunchOptions = JSON.parse(
        JSON.stringify(config.options || {}),
      );
      debugLog(id, "Base options cloned", {
        hasOptions: Object.keys(camoufoxOptions).length,
      });

      // Add profile path if provided
      if (config.profilePath) {
        camoufoxOptions.user_data_dir = config.profilePath;
        debugLog(id, "Set user_data_dir", { profilePath: config.profilePath });
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
      debugLog(id, "Set default options", {
        i_know_what_im_doing: true,
        disableTheming: true,
        showcursor: false,
      });

      // Generate fresh options for this specific instance
      debugLog(id, "Generating launch options via launchOptions function");
      const generatedOptions = await launchOptions(camoufoxOptions);
      debugLog(id, "Launch options generated successfully", {
        hasEnv: !!generatedOptions.env,
        argsLength: generatedOptions.args?.length || 0,
      });

      // Start with process environment to ensure proper inheritance
      let finalEnv = { ...process.env };
      debugLog(id, "Base environment variables set", {
        envVarCount: Object.keys(finalEnv).length,
      });

      // Add generated options environment variables
      if (generatedOptions.env) {
        finalEnv = { ...finalEnv, ...generatedOptions.env };
        debugLog(id, "Added generated environment variables", {
          generatedEnvCount: Object.keys(generatedOptions.env).length,
          totalEnvCount: Object.keys(finalEnv).length,
        });
      }

      // If we have a custom config from Rust, use it directly as environment variables
      if (config.customConfig) {
        debugLog(id, "Processing custom config", {
          customConfigLength: config.customConfig.length,
        });
        try {
          // Parse the custom config JSON string
          const customConfigObj = JSON.parse(config.customConfig);
          debugLog(id, "Custom config parsed successfully", {
            customConfigKeys: Object.keys(customConfigObj),
          });

          // Ensure default config values are preserved even with custom config
          const mergedConfig = {
            ...customConfigObj,
            disableTheming: true,
            showcursor: false,
            // allowAddonNewTab will be handled from the fingerprint config if present
          };

          // Convert merged config to environment variables using getEnvVars
          const customEnvVars = getEnvVars(mergedConfig);
          debugLog(id, "Custom config converted to environment variables", {
            customEnvVarCount: Object.keys(customEnvVars).length,
          });

          // Merge custom config with generated config (custom takes precedence)
          finalEnv = { ...finalEnv, ...customEnvVars };
          debugLog(id, "Custom config merged with final environment", {
            finalEnvCount: Object.keys(finalEnv).length,
          });
        } catch (error) {
          debugLog(id, "Failed to parse custom config", {
            error: error instanceof Error ? error.message : String(error),
          });
          console.error(
            `Camoufox worker ${id}: Failed to parse custom config, using generated config:`,
            error,
          );
          await gracefulShutdown();
          return;
        }
      } else {
        debugLog(id, "No custom config provided");
      }
      // Prepare profile path for persistent context
      const profilePath = config.profilePath || "";
      debugLog(id, "Profile path prepared", { profilePath });

      // Launch persistent context with the final configuration
      const finalOptions: any = {
        ...generatedOptions,
        env: finalEnv,
      };
      debugLog(id, "Final launch options prepared", {
        hasExecutablePath: !!finalOptions.executablePath,
        hasProxy: !!camoufoxOptions.proxy,
        profilePath,
      });

      // If a custom executable path was provided, ensure Playwright uses it
      if (
        (camoufoxOptions as any).executable_path &&
        typeof (camoufoxOptions as any).executable_path === "string"
      ) {
        finalOptions.executablePath = (camoufoxOptions as any)
          .executable_path as string;
        debugLog(id, "Custom executable path set", {
          executablePath: finalOptions.executablePath,
        });
      }

      // Only add proxy if it exists and is valid
      if (camoufoxOptions.proxy) {
        debugLog(id, "Processing proxy configuration", {
          proxyString: camoufoxOptions.proxy,
        });
        try {
          finalOptions.proxy = parseProxyString(camoufoxOptions.proxy);
          debugLog(id, "Proxy parsed successfully");
        } catch (error) {
          debugLog(id, "Failed to parse proxy", {
            error: error instanceof Error ? error.message : String(error),
          });
          console.error({
            message: "Failed to parse proxy, launching without proxy",
            error,
          });
          await gracefulShutdown();
          return;
        }
      }

      // Use launchPersistentContext instead of launchServer
      debugLog(id, "Launching persistent context", { profilePath });
      context = await firefox.launchPersistentContext(
        profilePath,
        finalOptions,
      );
      debugLog(id, "Persistent context launched successfully");

      // Get the browser instance from context
      browser = context.browser();
      debugLog(id, "Browser instance obtained from context", {
        browserConnected: browser?.isConnected(),
      });

      // Handle browser disconnection for proper cleanup
      if (browser) {
        browser.on("disconnected", () => void gracefulShutdown());
        debugLog(id, "Browser disconnect handler registered");
      }

      // Handle context close for proper cleanup
      context.on("close", () => void gracefulShutdown());
      debugLog(id, "Context close handler registered");

      saveCamoufoxConfig(config);

      // Monitor for window closure
      const startWindowMonitoring = () => {
        debugLog(id, "Starting window monitoring");
        windowCheckInterval = setInterval(async () => {
          try {
            // Check if context is still active
            if (!context?.pages || context.pages().length === 0) {
              debugLog(id, "No pages found in context, shutting down");
              if (windowCheckInterval) {
                clearInterval(windowCheckInterval);
              }
              await gracefulShutdown();
              return;
            }

            // Check if browser is still connected (if available)
            if (browser && !browser.isConnected()) {
              debugLog(id, "Browser disconnected, shutting down");
              if (windowCheckInterval) {
                clearInterval(windowCheckInterval);
              }
              await gracefulShutdown();
              return;
            }

            // Check pages in the persistent context
            const pages = context.pages();
            if (pages.length === 0) {
              debugLog(id, "No pages in context, shutting down");
              if (windowCheckInterval) {
                clearInterval(windowCheckInterval);
              }
              await gracefulShutdown();
            }
          } catch (error) {
            debugLog(id, "Error in window monitoring", {
              error: error instanceof Error ? error.message : String(error),
            });
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
        debugLog(id, "Opening URL in browser", { url: config.url });
        try {
          const pages = await context.pages();
          if (pages.length) {
            const page = pages[0];
            debugLog(id, "Navigating to URL");
            await page.goto(config.url, {
              waitUntil: "domcontentloaded",
              timeout: 30000,
            });
            debugLog(id, "URL opened successfully");

            // Start monitoring after page is created
            startWindowMonitoring();
          } else {
            debugLog(id, "No pages available to open URL");
            startWindowMonitoring();
          }
        } catch (urlError) {
          debugLog(id, "Failed to open URL", {
            error:
              urlError instanceof Error ? urlError.message : String(urlError),
          });
          console.error({
            message: "Failed to open URL",
            error: urlError,
          });
          // URL opening failure doesn't affect startup success
          // Still start monitoring
          startWindowMonitoring();
        }
      } else {
        debugLog(id, "No URL provided, starting monitoring");
        // Start monitoring after page is created
        startWindowMonitoring();
      }

      // Monitor browser/context connection
      debugLog(id, "Starting keep-alive monitoring");
      const keepAlive = setInterval(async () => {
        try {
          // Check if context is still active
          if (!context?.pages) {
            debugLog(id, "Context not active in keep-alive, shutting down");
            clearInterval(keepAlive);
            await gracefulShutdown();
            return;
          }

          // Check browser connection if available
          if (browser && !browser.isConnected()) {
            debugLog(id, "Browser not connected in keep-alive, shutting down");
            clearInterval(keepAlive);
            await gracefulShutdown();
            return;
          }
        } catch (error) {
          debugLog(id, "Error in keep-alive check", {
            error: error instanceof Error ? error.message : String(error),
          });
          console.error({
            message: "Error in keepAlive check",
            error,
          });
          clearInterval(keepAlive);
          await gracefulShutdown();
        }
      }, 2000);
    } catch (error) {
      debugLog(id, "Failed to launch Camoufox", {
        error: error instanceof Error ? error.message : String(error),
      });
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
