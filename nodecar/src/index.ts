import type { LaunchOptions } from "camoufox-js/dist/utils.js";
import { program } from "commander";
import {
  startCamoufoxProcess,
  stopAllCamoufoxProcesses,
  stopCamoufoxProcess,
} from "./camoufox-launcher.js";
import { listCamoufoxConfigs } from "./camoufox-storage.js";
import { runCamoufoxWorker } from "./camoufox-worker.js";
import {
  startProxyProcess,
  stopAllProxyProcesses,
  stopProxyProcess,
} from "./proxy-runner";
import { listProxyConfigs } from "./proxy-storage";
import { runProxyWorker } from "./proxy-worker";

// Command for proxy management
program
  .command("proxy")
  .argument("<action>", "start, stop, or list proxies")
  .option("--host <host>", "upstream proxy host")
  .option("--proxy-port <port>", "upstream proxy port", Number.parseInt)
  .option("--type <type>", "proxy type (http, https, socks4, socks5)")
  .option("--username <username>", "proxy username")
  .option("--password <password>", "proxy password")
  .option(
    "-p, --port <number>",
    "local port to use (random if not specified)",
    Number.parseInt,
  )
  .option("--ignore-certificate", "ignore certificate errors for HTTPS proxies")
  .option("--id <id>", "proxy ID for stop command")
  .option(
    "-u, --upstream <url>",
    "upstream proxy URL (protocol://[username:password@]host:port)",
  )
  .description("manage proxy servers")
  .action(
    async (
      action: string,
      options: {
        host?: string;
        proxyPort?: number;
        type?: string;
        username?: string;
        password?: string;
        port?: number;
        ignoreCertificate?: boolean;
        id?: string;
        upstream?: string;
      },
    ) => {
      if (action === "start") {
        let upstreamUrl: string | undefined;

        // Build upstream URL from individual components if provided
        if (options.host && options.proxyPort && options.type) {
          const protocol =
            options.type === "socks4" || options.type === "socks5"
              ? options.type
              : "http";
          const auth =
            options.username && options.password
              ? `${encodeURIComponent(options.username)}:${encodeURIComponent(
                  options.password,
                )}@`
              : "";
          upstreamUrl = `${protocol}://${auth}${options.host}:${options.proxyPort}`;
        } else if (options.upstream) {
          upstreamUrl = options.upstream;
        }
        // If no upstream is provided, create a direct proxy

        try {
          const config = await startProxyProcess(upstreamUrl, {
            port: options.port,
            ignoreProxyCertificate: options.ignoreCertificate,
          });

          // Output the configuration as JSON for the Rust side to parse
          console.log(
            JSON.stringify({
              id: config.id,
              localPort: config.localPort,
              localUrl: config.localUrl,
              upstreamUrl: config.upstreamUrl,
            }),
          );

          // Exit successfully to allow the process to detach
          process.exit(0);
        } catch (error: unknown) {
          console.error(
            `Failed to start proxy: ${
              error instanceof Error ? error.message : JSON.stringify(error)
            }`,
          );
          process.exit(1);
        }
      } else if (action === "stop") {
        if (options.id) {
          const stopped = await stopProxyProcess(options.id);
          console.log(JSON.stringify({ success: stopped }));
        } else if (options.upstream) {
          // Find proxies with this upstream URL
          const configs = listProxyConfigs().filter(
            (config) => config.upstreamUrl === options.upstream,
          );

          if (configs.length === 0) {
            console.error(`No proxies found for ${options.upstream}`);
            process.exit(1);
            return;
          }

          for (const config of configs) {
            const stopped = await stopProxyProcess(config.id);
            console.log(JSON.stringify({ success: stopped }));
          }
        } else {
          await stopAllProxyProcesses();
          console.log(JSON.stringify({ success: true }));
        }
        process.exit(0);
      } else if (action === "list") {
        const configs = listProxyConfigs();
        console.log(JSON.stringify(configs));
        process.exit(0);
      } else {
        console.error("Invalid action. Use 'start', 'stop', or 'list'");
        process.exit(1);
      }
    },
  );

// Command for proxy worker (internal use)
program
  .command("proxy-worker")
  .argument("<action>", "start a proxy worker")
  .requiredOption("--id <id>", "proxy configuration ID")
  .description("run a proxy worker process")
  .action(async (action: string, options: { id: string }) => {
    if (action === "start") {
      await runProxyWorker(options.id);
    } else {
      console.error("Invalid action for proxy-worker. Use 'start'");
      process.exit(1);
    }
  });

// Command for Camoufox management
program
  .command("camoufox")
  .argument("<action>", "start, stop, or list Camoufox instances")
  .option("--id <id>", "Camoufox ID for stop command")
  .option("--profile-path <path>", "profile directory path")
  .option("--url <url>", "URL to open")

  // Operating system fingerprinting
  .option(
    "--os <os>",
    "OS to emulate (windows, macos, linux, or comma-separated list)",
  )

  // Blocking options
  .option("--block-images", "block all images")
  .option("--block-webrtc", "block WebRTC entirely")
  .option("--block-webgl", "block WebGL")

  // Security options
  .option("--disable-coop", "disable Cross-Origin-Opener-Policy")

  // Geolocation and IP
  .option(
    "--geoip <ip>",
    "IP address for geolocation spoofing (or 'auto' for automatic)",
  )
  .option("--country <country>", "country code for geolocation")
  .option("--timezone <timezone>", "timezone to spoof")
  .option("--latitude <lat>", "latitude for geolocation", parseFloat)
  .option("--longitude <lng>", "longitude for geolocation", parseFloat)

  // UI and behavior
  .option(
    "--humanize [duration]",
    "humanize cursor movement (optional max duration in seconds)",
    (val) => (val ? parseFloat(val) : true),
  )
  .option("--headless", "run in headless mode")

  // Localization
  .option("--locale <locale>", "locale(s) to use (comma-separated)")

  // Extensions and fonts
  .option("--addons <addons>", "Firefox addons to load (comma-separated paths)")
  .option("--fonts <fonts>", "additional fonts to load (comma-separated)")
  .option("--custom-fonts-only", "use only custom fonts, exclude OS fonts")
  .option(
    "--exclude-addons <addons>",
    "default addons to exclude (comma-separated)",
  )

  // Screen and window
  .option("--screen-min-width <width>", "minimum screen width", parseInt)
  .option("--screen-max-width <width>", "maximum screen width", parseInt)
  .option("--screen-min-height <height>", "minimum screen height", parseInt)
  .option("--screen-max-height <height>", "maximum screen height", parseInt)
  .option("--window-width <width>", "fixed window width", parseInt)
  .option("--window-height <height>", "fixed window height", parseInt)

  // Advanced options
  .option("--ff-version <version>", "Firefox version to emulate", parseInt)
  .option("--main-world-eval", "enable main world script evaluation")
  .option("--webgl-vendor <vendor>", "WebGL vendor string")
  .option("--webgl-renderer <renderer>", "WebGL renderer string")

  // Proxy
  .option(
    "--proxy <proxy>",
    "proxy URL (protocol://[username:password@]host:port)",
  )

  // Cache and performance
  .option("--disable-cache", "disable browser cache (cache enabled by default)")

  // Environment and debugging
  .option("--virtual-display <display>", "virtual display number (e.g., :99)")
  .option("--debug", "enable debug output")
  .option("--args <args>", "additional browser arguments (comma-separated)")
  .option("--env <env>", "environment variables (JSON string)")

  // Firefox preferences
  .option("--firefox-prefs <prefs>", "Firefox user preferences (JSON string)")

  // Note: theming and cursor options are hardcoded and not user-configurable

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

          // Geolocation - always enable geoip for proper spoofing
          if (options.geoip) {
            camoufoxOptions.geoip =
              options.geoip === "auto" ? true : (options.geoip as string);
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
              return;
            }
          }

          const config = await startCamoufoxProcess(
            camoufoxOptions,
            typeof options.profilePath === "string"
              ? options.profilePath
              : undefined,
            typeof options.url === "string" ? options.url : undefined,
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
      } else {
        console.error("Invalid action. Use 'start', 'stop', or 'list'");
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
      console.error("Invalid action for camoufox-worker. Use 'start'");
      process.exit(1);
    }
  });

program.parse();
