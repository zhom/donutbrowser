import { launchServer } from "camoufox-js";
import { type Browser, type BrowserServer, firefox } from "playwright-core";
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
      // Prepare options for Camoufox
      const camoufoxOptions = { ...config.options };

      // Add profile path if provided
      if (config.profilePath) {
        camoufoxOptions.user_data_dir = config.profilePath;
      }

      // Theming
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

      // Launch the server with proper options
      server = await launchServer({
        ws_path: `/ws_${config.id}`,
        os: camoufoxOptions.os,
        block_images: camoufoxOptions.block_images,
        block_webrtc: camoufoxOptions.block_webrtc,
        block_webgl: camoufoxOptions.block_webgl,
        disable_coop: camoufoxOptions.disable_coop,
        geoip: camoufoxOptions.geoip,
        humanize: camoufoxOptions.humanize,
        locale: camoufoxOptions.locale,
        addons: camoufoxOptions.addons,
        fonts: camoufoxOptions.fonts,
        custom_fonts_only: camoufoxOptions.custom_fonts_only,
        exclude_addons: camoufoxOptions.exclude_addons,
        screen: camoufoxOptions.screen,
        window: camoufoxOptions.window,
        fingerprint: camoufoxOptions.fingerprint,
        ff_version: camoufoxOptions.ff_version,
        headless: camoufoxOptions.headless,
        main_world_eval: camoufoxOptions.main_world_eval,
        executable_path: camoufoxOptions.executable_path,
        firefox_user_prefs: camoufoxOptions.firefox_user_prefs,
        proxy: camoufoxOptions.proxy,
        enable_cache: camoufoxOptions.enable_cache,
        args: camoufoxOptions.args,
        env: camoufoxOptions.env,
        debug: camoufoxOptions.debug,
        virtual_display: camoufoxOptions.virtual_display,
        webgl_config: camoufoxOptions.webgl_config,
        config: {
          disableTheming: true,
          showcursor: false,
          timezone: camoufoxOptions.timezone,
        },
      });

      // Connect to the server
      browser = await firefox.connect(server.wsEndpoint());
      const context = await browser.newContext();

      // Handle browser disconnection for proper cleanup
      browser.on("disconnected", () => void gracefulShutdown());

      saveCamoufoxConfig(config);

      // Monitor for window closure to handle Command+Q properly

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
