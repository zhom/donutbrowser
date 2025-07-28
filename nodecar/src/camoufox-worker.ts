import { launchServer } from "camoufox-js";
import getPort from "get-port";
import type { Page } from "playwright-core";
import { firefox } from "playwright-core";
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

  let server: Awaited<ReturnType<typeof launchServer>> | null = null;
  let browser: Awaited<ReturnType<typeof firefox.connect>> | null = null;

  // Handle process termination gracefully
  const gracefulShutdown = async () => {
    try {
      if (browser) {
        await browser.close();
      }
      if (server) {
        await server.close();
      }
    } catch {
      // Ignore errors during shutdown
    }
    process.exit(0);
  };

  process.on("SIGTERM", () => void gracefulShutdown());
  process.on("SIGINT", () => void gracefulShutdown());

  // Handle uncaught exceptions
  process.on("uncaughtException", (error) => {
    console.error(
      JSON.stringify({
        error: "Uncaught exception",
        message: error.message,
        stack: error.stack,
        id: id,
      }),
    );
    process.exit(1);
  });

  process.on("unhandledRejection", (reason) => {
    console.error(
      JSON.stringify({
        error: "Unhandled rejection",
        reason: String(reason),
        id: id,
      }),
    );
    process.exit(1);
  });

  // Add a timeout to prevent hanging
  const startupTimeout = setTimeout(() => {
    console.error(
      JSON.stringify({
        error: "Startup timeout",
        message: "Worker startup timeout after 30 seconds",
        id: id,
      }),
    );
    process.exit(1);
  }, 30000);

  // Start the browser server
  try {
    const port = await getPort();

    // Prepare options for Camoufox
    const camoufoxOptions = { ...config.options };

    // Add profile path if provided
    if (config.profilePath) {
      camoufoxOptions.user_data_dir = config.profilePath;
    }

    camoufoxOptions.disableTheming = true;
    camoufoxOptions.showcursor = false;

    // Don't force headless mode - let the user configuration decide
    if (camoufoxOptions.headless === undefined) {
      camoufoxOptions.headless = false; // Default to visible for debugging
    }

    try {
      // Launch Camoufox server
      server = await launchServer({
        ...camoufoxOptions,
        port: port,
        ws_path: "/camoufox",
      });
    } catch (error) {
      console.error(
        JSON.stringify({
          error: "Failed to launch Camoufox server",
          message: error instanceof Error ? error.message : String(error),
          id: id,
        }),
      );
      process.exit(1);
    }

    if (!server) {
      console.error(
        JSON.stringify({
          error: "Failed to launch Camoufox server",
          message:
            "Camoufox is not installed. Please install Camoufox first by running: npx camoufox-js fetch",
          id: id,
        }),
      );
      process.exit(1);
    }

    // Connect to the server
    try {
      browser = await firefox.connect(server.wsEndpoint());
    } catch (error) {
      console.error(
        JSON.stringify({
          error: "Failed to connect to Camoufox server",
          message: error instanceof Error ? error.message : String(error),
          id: id,
        }),
      );
      process.exit(1);
    }

    // Update config with server details
    config.port = port;
    config.wsEndpoint = server.wsEndpoint();
    saveCamoufoxConfig(config);

    // Clear the startup timeout since we succeeded
    clearTimeout(startupTimeout);

    // Output success JSON for the parent process
    console.log(
      JSON.stringify({
        success: true,
        id: id,
        port: port,
        wsEndpoint: server.wsEndpoint(),
        message: "Camoufox server started successfully",
      }),
    );

    // Open URL if provided
    if (config.url) {
      try {
        const page: Page = await browser.newPage();
        await page.goto(config.url);
      } catch (error) {
        // Don't exit here, just log the error as JSON
        console.error(
          JSON.stringify({
            error: "Failed to open URL",
            url: config.url,
            message: error instanceof Error ? error.message : String(error),
            id: id,
          }),
        );
      }
    } else {
      // If no URL is provided, create a blank page to keep the browser alive
      try {
        await browser.newPage();
      } catch (error) {
        console.error(
          JSON.stringify({
            error: "Failed to create blank page",
            message: error instanceof Error ? error.message : String(error),
            id: id,
          }),
        );
      }
    }

    // Keep the process alive by waiting for the browser to disconnect
    browser.on("disconnected", () => {
      process.exit(0);
    });

    // Keep the process alive with a simple check
    const keepAlive = setInterval(async () => {
      try {
        // Check if browser is still connected
        if (!browser || !browser.isConnected()) {
          clearInterval(keepAlive);
          process.exit(0);
        }
      } catch (error) {
        // If we can't check the connection, assume it's dead
        clearInterval(keepAlive);
        process.exit(0);
      }
    }, 5000);

    // Handle process staying alive
    process.stdin.resume();
  } catch (error) {
    clearTimeout(startupTimeout);
    console.error(
      JSON.stringify({
        error: "Failed to start Camoufox worker",
        message: error instanceof Error ? error.message : String(error),
        stack: error instanceof Error ? error.stack : undefined,
        config: config,
        id: id,
      }),
    );
    process.exit(1);
  }
}
