import { program } from "commander";
import type { LaunchOptions } from "donutbrowser-camoufox-js/dist/utils.js";
import {
  generateCamoufoxConfig,
  startCamoufoxProcess,
  stopAllCamoufoxProcesses,
  stopCamoufoxProcess,
} from "./camoufox-launcher.js";
import { listCamoufoxConfigs } from "./camoufox-storage.js";
import { runCamoufoxWorker } from "./camoufox-worker.js";

// Command for Camoufox management
program
  .command("camoufox")
  .argument(
    "<action>",
    "start, stop, list, or generate-config Camoufox instances",
  )
  .option("--id <id>", "Camoufox ID for stop command")
  .option("--profile-path <path>", "profile directory path")
  .option("--url <url>", "URL to open")

  // Config generation options
  .option("--proxy <proxy>", "proxy URL for config generation")
  .option("--max-width <width>", "maximum screen width", parseInt)
  .option("--max-height <height>", "maximum screen height", parseInt)
  .option("--min-width <width>", "minimum screen width", parseInt)
  .option("--min-height <height>", "minimum screen height", parseInt)
  .option("--geoip", "enable geoip")
  .option("--block-images", "block images")
  .option("--block-webrtc", "block WebRTC")
  .option("--block-webgl", "block WebGL")
  .option("--executable-path <path>", "executable path")
  .option("--fingerprint <json>", "fingerprint JSON string")
  .option("--headless", "run in headless mode")
  .option("--custom-config <json>", "custom config JSON string")
  .option(
    "--os <os>",
    "operating system for fingerprint: windows, macos, linux",
  )

  .description("manage Camoufox browser instances")
  .action(
    async (
      action: string,
      options: Record<string, string | number | boolean | undefined>,
    ) => {
      if (action === "start") {
        try {
          // Build Camoufox options in the format expected by camoufox-js
          const camoufoxOptions: LaunchOptions = {};

          // OS fingerprinting
          if (options.os && typeof options.os === "string") {
            camoufoxOptions.os = options.os.includes(",")
              ? (options.os.split(",") as ("windows" | "macos" | "linux")[])
              : (options.os as "windows" | "macos" | "linux");
          }

          // Blocking options
          if (options.blockImages) camoufoxOptions.block_images = true;
          if (options.blockWebrtc) camoufoxOptions.block_webrtc = true;
          if (options.blockWebgl) camoufoxOptions.block_webgl = true;

          // Security options
          if (options.disableCoop) camoufoxOptions.disable_coop = true;

          if (options.geoip) {
            camoufoxOptions.geoip = true;
          }

          if (options.latitude && options.longitude) {
            camoufoxOptions.geolocation = {
              latitude: options.latitude as number,
              longitude: options.longitude as number,
              accuracy: 100,
            };
          }
          if (options.country)
            camoufoxOptions.country = options.country as string;
          if (options.timezone)
            camoufoxOptions.timezone = options.timezone as string;

          if (options.humanize)
            camoufoxOptions.humanize = options.humanize as boolean;
          if (options.headless) camoufoxOptions.headless = true;

          // Localization
          if (options.locale && typeof options.locale === "string") {
            camoufoxOptions.locale = options.locale.includes(",")
              ? options.locale.split(",")
              : options.locale;
          }

          // Extensions and fonts
          if (options.addons && typeof options.addons === "string")
            camoufoxOptions.addons = options.addons.split(",");
          if (options.fonts && typeof options.fonts === "string")
            camoufoxOptions.fonts = options.fonts.split(",");
          if (options.customFontsOnly) camoufoxOptions.custom_fonts_only = true;
          if (
            options.excludeAddons &&
            typeof options.excludeAddons === "string"
          )
            camoufoxOptions.exclude_addons = options.excludeAddons.split(
              ",",
            ) as "UBO"[];

          // Executable path: forward through to camoufox-js and ultimately Playwright
          if (
            options.executablePath &&
            typeof options.executablePath === "string"
          ) {
            // camoufox-js uses snake_case for this option
            (camoufoxOptions as any).executable_path =
              options.executablePath as string;
          }

          // Screen and window
          const screen: {
            minWidth?: number;
            maxWidth?: number;
            minHeight?: number;
            maxHeight?: number;
          } = {};
          if (options.screenMinWidth)
            screen.minWidth = options.screenMinWidth as number;
          if (options.screenMaxWidth)
            screen.maxWidth = options.screenMaxWidth as number;
          if (options.screenMinHeight)
            screen.minHeight = options.screenMinHeight as number;
          if (options.screenMaxHeight)
            screen.maxHeight = options.screenMaxHeight as number;
          if (Object.keys(screen).length > 0) camoufoxOptions.screen = screen;

          if (options.windowWidth && options.windowHeight) {
            camoufoxOptions.window = [
              options.windowWidth as number,
              options.windowHeight as number,
            ];
          }

          // Advanced options
          if (options.ffVersion)
            camoufoxOptions.ff_version = options.ffVersion as number;
          if (options.mainWorldEval) camoufoxOptions.main_world_eval = true;
          if (options.webglVendor && options.webglRenderer) {
            camoufoxOptions.webgl_config = [
              options.webglVendor as string,
              options.webglRenderer as string,
            ];
          }

          // Proxy
          if (options.proxy) camoufoxOptions.proxy = options.proxy as string;

          // Cache and performance - default to enabled
          camoufoxOptions.enable_cache = !options.disableCache;

          // Environment and debugging
          if (options.virtualDisplay)
            camoufoxOptions.virtual_display = options.virtualDisplay as string;
          if (options.debug) camoufoxOptions.debug = true;

          // Handle headless mode via flag instead of environment variable
          if (options.headless) {
            camoufoxOptions.headless = true;
          }
          if (options.args && typeof options.args === "string")
            camoufoxOptions.args = options.args.split(",");
          if (options.env && typeof options.env === "string") {
            try {
              camoufoxOptions.env = JSON.parse(options.env);
            } catch (e) {
              console.error(
                JSON.stringify({
                  error: "Invalid JSON for --env option",
                  message: String(e),
                }),
              );
              process.exit(1);
              return;
            }
          }

          // Firefox preferences
          if (
            options.firefoxPrefs &&
            typeof options.firefoxPrefs === "string"
          ) {
            try {
              camoufoxOptions.firefox_user_prefs = JSON.parse(
                options.firefoxPrefs,
              );
            } catch (e) {
              console.error(
                JSON.stringify({
                  error: "Invalid JSON for --firefox-prefs option",
                  message: String(e),
                }),
              );
              process.exit(1);
            }
          }

          const config = await startCamoufoxProcess(
            camoufoxOptions,
            typeof options.profilePath === "string"
              ? options.profilePath
              : undefined,
            typeof options.url === "string" ? options.url : undefined,
            typeof options.customConfig === "string"
              ? options.customConfig
              : undefined,
          );

          console.log(
            JSON.stringify({
              id: config.id,
              processId: config.processId,
              profilePath: config.profilePath,
              url: config.url,
            }),
          );

          process.exit(0);
        } catch (error: unknown) {
          console.error(
            JSON.stringify({
              error: "Failed to start Camoufox",
              message: error instanceof Error ? error.message : String(error),
            }),
          );
          process.exit(1);
        }
      } else if (action === "stop") {
        if (options.id && typeof options.id === "string") {
          const stopped = await stopCamoufoxProcess(options.id);
          console.log(JSON.stringify({ success: stopped }));
        } else {
          await stopAllCamoufoxProcesses();
          console.log(JSON.stringify({ success: true }));
        }
        process.exit(0);
      } else if (action === "list") {
        const configs = listCamoufoxConfigs();
        console.log(JSON.stringify(configs));
        process.exit(0);
      } else if (action === "generate-config") {
        try {
          const config = await generateCamoufoxConfig({
            proxy:
              typeof options.proxy === "string" ? options.proxy : undefined,
            maxWidth:
              typeof options.maxWidth === "number"
                ? options.maxWidth
                : undefined,
            maxHeight:
              typeof options.maxHeight === "number"
                ? options.maxHeight
                : undefined,
            minWidth:
              typeof options.minWidth === "number"
                ? options.minWidth
                : undefined,
            minHeight:
              typeof options.minHeight === "number"
                ? options.minHeight
                : undefined,
            geoip: Boolean(options.geoip),
            blockImages:
              typeof options.blockImages === "boolean"
                ? options.blockImages
                : undefined,
            blockWebrtc:
              typeof options.blockWebrtc === "boolean"
                ? options.blockWebrtc
                : undefined,
            blockWebgl:
              typeof options.blockWebgl === "boolean"
                ? options.blockWebgl
                : undefined,
            executablePath:
              typeof options.executablePath === "string"
                ? options.executablePath
                : undefined,
            fingerprint:
              typeof options.fingerprint === "string"
                ? options.fingerprint
                : undefined,
            os:
              typeof options.os === "string"
                ? (options.os as "windows" | "macos" | "linux")
                : undefined,
          });
          console.log(config);
          process.exit(0);
        } catch (error: unknown) {
          console.error({
            error: "Failed to generate config",
            message:
              error instanceof Error ? error.message : JSON.stringify(error),
          });
          process.exit(1);
        }
      } else {
        console.error({
          error: "Invalid action",
          message: "Use 'start', 'stop', 'list', or 'generate-config'",
        });
        process.exit(1);
      }
    },
  );

// Command for Camoufox worker (internal use)
program
  .command("camoufox-worker")
  .argument("<action>", "start a Camoufox worker")
  .requiredOption("--id <id>", "Camoufox configuration ID")
  .description("run a Camoufox worker process")
  .action(async (action: string, options: { id: string }) => {
    if (action === "start") {
      await runCamoufoxWorker(options.id);
    } else {
      console.error({
        error: "Invalid action for camoufox-worker",
        message: "Use 'start'",
      });
      process.exit(1);
    }
  });

program.parse();
