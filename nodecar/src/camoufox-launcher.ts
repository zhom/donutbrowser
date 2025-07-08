import { spawn } from "child_process";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

export interface CamoufoxConfig {
  id: string;
  pid?: number;
  executablePath: string;
  profilePath: string;
  url?: string;
  options: CamoufoxLaunchOptions;
}

export interface CamoufoxLaunchOptions {
  // Operating system to use for fingerprint generation
  os?: "windows" | "macos" | "linux" | string[];

  // Blocking options
  block_images?: boolean;
  block_webrtc?: boolean;
  block_webgl?: boolean;

  // Security options
  disable_coop?: boolean;

  // Geolocation options
  geoip?: string | boolean;

  // UI behavior
  humanize?: boolean | number;

  // Localization
  locale?: string | string[];

  // Extensions and fonts
  addons?: string[];
  fonts?: string[];
  custom_fonts_only?: boolean;
  exclude_addons?: string[];

  // Screen and window
  screen?: {
    minWidth?: number;
    maxWidth?: number;
    minHeight?: number;
    maxHeight?: number;
  };
  window?: [number, number];

  // Fingerprint
  fingerprint?: any;

  // Version and mode
  ff_version?: number;
  headless?: boolean;
  main_world_eval?: boolean;

  // Custom executable path
  executable_path?: string;

  // Firefox preferences
  firefox_user_prefs?: Record<string, any>;

  // Proxy settings
  proxy?:
    | string
    | {
        server: string;
        username?: string;
        password?: string;
        bypass?: string;
      };

  // Cache and performance
  enable_cache?: boolean;

  // Additional options
  args?: string[];
  env?: Record<string, string | number | boolean>;
  debug?: boolean;
  virtual_display?: string;
  webgl_config?: [string, string];

  // Custom options
  timezone?: string;
  country?: string;
  geolocation?: {
    latitude: number;
    longitude: number;
    accuracy?: number;
  };
}

// Store for active Camoufox processes
const activeCamoufoxProcesses = new Map<string, CamoufoxConfig>();

/**
 * Generate a unique ID for the Camoufox instance
 */
function generateCamoufoxId(): string {
  return `camoufox_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
}

/**
 * Save Camoufox configuration to storage
 */
function saveCamoufoxConfig(config: CamoufoxConfig): void {
  try {
    const configDir = path.join(os.tmpdir(), "nodecar_camoufox");
    if (!fs.existsSync(configDir)) {
      fs.mkdirSync(configDir, { recursive: true });
    }

    const configFile = path.join(configDir, `${config.id}.json`);
    fs.writeFileSync(configFile, JSON.stringify(config, null, 2));
    activeCamoufoxProcesses.set(config.id, config);
  } catch (error) {
    console.error(`Failed to save Camoufox config: ${error}`);
  }
}

/**
 * Load Camoufox configuration from storage
 */
function loadCamoufoxConfig(id: string): CamoufoxConfig | null {
  try {
    const configFile = path.join(os.tmpdir(), "nodecar_camoufox", `${id}.json`);
    if (fs.existsSync(configFile)) {
      const config = JSON.parse(fs.readFileSync(configFile, "utf8"));
      activeCamoufoxProcesses.set(id, config);
      return config;
    }
  } catch (error) {
    console.error(`Failed to load Camoufox config: ${error}`);
  }
  return null;
}

/**
 * Delete Camoufox configuration from storage
 */
function deleteCamoufoxConfig(id: string): boolean {
  try {
    const configFile = path.join(os.tmpdir(), "nodecar_camoufox", `${id}.json`);
    if (fs.existsSync(configFile)) {
      fs.unlinkSync(configFile);
    }
    activeCamoufoxProcesses.delete(id);
    return true;
  } catch (error) {
    console.error(`Failed to delete Camoufox config: ${error}`);
    return false;
  }
}

/**
 * Load all Camoufox configurations on startup
 */
function loadAllCamoufoxConfigs(): void {
  try {
    const configDir = path.join(os.tmpdir(), "nodecar_camoufox");
    if (fs.existsSync(configDir)) {
      const files = fs.readdirSync(configDir);
      for (const file of files) {
        if (file.endsWith(".json")) {
          const id = path.basename(file, ".json");
          loadCamoufoxConfig(id);
        }
      }
    }
  } catch (error) {
    console.error(`Failed to load Camoufox configs: ${error}`);
  }
}

/**
 * Check if a process is still running
 */
function isProcessRunning(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return false;
  }
}

/**
 * Convert Camoufox options to command line arguments
 */
function buildCamoufoxArgs(
  options: CamoufoxLaunchOptions,
  profilePath: string,
  url?: string,
): string[] {
  const args: string[] = [];

  // Always use profile
  args.push("-profile", profilePath);

  // Cache enabled by default as requested
  if (options.enable_cache !== false) {
    // Cache is enabled by default in Camoufox, no special args needed
  }

  // Headless mode
  if (options.headless) {
    args.push("-headless");
  }

  // No remote for security (anti-detect)
  args.push("-no-remote");

  // Custom Firefox user preferences will be written to user.js in profile

  // Additional custom args
  if (options.args) {
    args.push(...options.args);
  }

  // URL to open
  if (url) {
    args.push(url);
  }

  return args;
}

/**
 * Create Camoufox configuration object from launch options
 * This follows the complete Camoufox schema for CAMOU_CONFIG_* environment variables
 */
function createCamoufoxConfig(options: CamoufoxLaunchOptions): any {
  const config: any = {};

  // Debug flag
  if (options.debug !== undefined) {
    config.debug = options.debug;
  }

  // Locale settings - parse locale string into language and region
  if (options.locale) {
    const localeValue = Array.isArray(options.locale)
      ? options.locale[0]
      : options.locale;

    // Parse locale like "en-US" into language and region
    const localeParts = localeValue.split("-");
    if (localeParts.length >= 2) {
      config["locale:language$__LOCALE"] = localeParts[0];
      config["locale:region$__LOCALE"] = localeParts[1];
    } else {
      config["locale:language$__LOCALE"] = localeParts[0];
      // Default region if not specified
      config["locale:region$__LOCALE"] = "US";
    }

    // Set navigator language properties
    config["navigator.language"] = localeValue;
    config["navigator.languages"] = Array.isArray(options.locale)
      ? options.locale
      : [localeValue];
    config["headers.Accept-Language"] = localeValue;
    config["locale:all"] = localeValue;
  }

  // Screen dimensions from screen options
  if (options.screen) {
    if (options.screen.maxWidth) {
      config["screen.width$__SC"] = options.screen.maxWidth;
      config["screen.availWidth$__SC"] = options.screen.maxWidth;
    }
    if (options.screen.maxHeight) {
      config["screen.height$__SC"] = options.screen.maxHeight;
      config["screen.availHeight$__SC"] = options.screen.maxHeight;
    }

    // Set default screen properties if not specified
    if (!options.screen.maxWidth) {
      config["screen.width$__SC"] = 1920;
      config["screen.availWidth$__SC"] = 1920;
    }
    if (!options.screen.maxHeight) {
      config["screen.height$__SC"] = 1080;
      config["screen.availHeight$__SC"] = 1080;
    }

    // Default screen position and color depth
    config["screen.availTop"] = 0;
    config["screen.availLeft"] = 0;
    config["screen.colorDepth"] = 24;
    config["screen.pixelDepth"] = 24;
  } else {
    // Default screen settings if not specified
    config["screen.width$__SC"] = 1920;
    config["screen.height$__SC"] = 1080;
    config["screen.availWidth$__SC"] = 1920;
    config["screen.availHeight$__SC"] = 1080;
    config["screen.availTop"] = 0;
    config["screen.availLeft"] = 0;
    config["screen.colorDepth"] = 24;
    config["screen.pixelDepth"] = 24;
  }

  // Window dimensions
  if (options.window) {
    config["window.outerWidth$__W_OUTER"] = options.window[0];
    config["window.outerHeight$__W_OUTER"] = options.window[1];
    config["window.innerWidth$__W_INNER"] = options.window[0] - 16; // Account for scrollbars
    config["window.innerHeight$__W_INNER"] = options.window[1] - 100; // Account for browser chrome
  } else {
    // Default window dimensions
    config["window.outerWidth$__W_OUTER"] = 1280;
    config["window.outerHeight$__W_OUTER"] = 720;
    config["window.innerWidth$__W_INNER"] = 1264;
    config["window.innerHeight$__W_INNER"] = 620;
  }

  // Window position and properties
  config["window.screenX"] = 0;
  config["window.screenY"] = 0;
  config["window.devicePixelRatio"] = 1.0;
  config["window.scrollMinX"] = 0;
  config["window.scrollMinY"] = 0;
  config["window.scrollMaxX"] = 0;
  config["window.scrollMaxY"] = 0;
  config["screen.pageXOffset"] = 0.0;
  config["screen.pageYOffset"] = 0.0;

  // Document body dimensions
  config["document.body.clientWidth$__DOC_BODY"] =
    config["window.innerWidth$__W_INNER"];
  config["document.body.clientHeight$__DOC_BODY"] =
    config["window.innerHeight$__W_INNER"];
  config["document.body.clientTop"] = 0;
  config["document.body.clientLeft"] = 0;

  // Geolocation
  if (options.geolocation) {
    config["geolocation:latitude$__GEO"] = options.geolocation.latitude;
    config["geolocation:longitude$__GEO"] = options.geolocation.longitude;
    if (options.geolocation.accuracy) {
      config["geolocation:accuracy"] = options.geolocation.accuracy;
    }
  }

  // Timezone
  if (options.timezone) {
    config.timezone = options.timezone;
  }

  // User Agent based on OS option
  const osOption = Array.isArray(options.os) ? options.os[0] : options.os;
  let userAgent: string;
  let platform: string;
  let oscpu: string;
  let appVersion: string;

  switch (osOption) {
    case "macos":
      userAgent =
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:135.0) Gecko/20100101 Firefox/135.0";
      platform = "MacIntel";
      oscpu = "Intel Mac OS X 10.15";
      appVersion =
        "5.0 (Macintosh; Intel Mac OS X 10.15; rv:135.0) Gecko/20100101 Firefox/135.0";
      break;
    case "linux":
      userAgent =
        "Mozilla/5.0 (X11; Linux x86_64; rv:135.0) Gecko/20100101 Firefox/135.0";
      platform = "Linux x86_64";
      oscpu = "Linux x86_64";
      appVersion =
        "5.0 (X11; Linux x86_64; rv:135.0) Gecko/20100101 Firefox/135.0";
      break;
    case "windows":
    default:
      userAgent =
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:135.0) Gecko/20100101 Firefox/135.0";
      platform = "Win32";
      oscpu = "Windows NT 10.0; Win64; x64";
      appVersion =
        "5.0 (Windows NT 10.0; Win64; x64; rv:135.0) Gecko/20100101 Firefox/135.0";
      break;
  }

  config["navigator.userAgent"] = userAgent;
  config["navigator.appVersion"] = appVersion;
  config["navigator.platform"] = platform;
  config["navigator.oscpu"] = oscpu;
  config["headers.User-Agent"] = userAgent;

  // Headers
  config["headers.Accept-Encoding"] = "gzip, deflate, br";

  // Fonts
  if (options.fonts && options.fonts.length > 0) {
    config.fonts = options.fonts;
  }
  config["fonts:spacing_seed"] = 0;

  // WebGL configuration
  if (
    options.webgl_config &&
    Array.isArray(options.webgl_config) &&
    options.webgl_config.length === 2
  ) {
    config["webGl:vendor$__WEBGL"] = options.webgl_config[0];
    config["webGl:renderer$__WEBGL"] = options.webgl_config[1];
  }

  // WebRTC IP spoofing from geoip
  if (
    options.geoip &&
    typeof options.geoip === "string" &&
    options.geoip !== "auto"
  ) {
    if (options.geoip.includes(":")) {
      // IPv6
      config["webrtc:ipv6"] = options.geoip;
      config["webrtc:localipv6"] = options.geoip;
    } else {
      // IPv4
      config["webrtc:ipv4"] = options.geoip;
      config["webrtc:localipv4"] = options.geoip;
    }
  }

  // Addons
  if (options.addons && options.addons.length > 0) {
    config.addons = options.addons;
  }

  // Humanization
  if (options.humanize !== undefined) {
    config.humanize = !!options.humanize;
    if (typeof options.humanize === "number") {
      config["humanize:maxTime"] = options.humanize;
      config["humanize:minTime"] = 0.0;
    } else {
      config["humanize:maxTime"] = 5.0;
      config["humanize:minTime"] = 0.5;
    }
  }

  // Cursor visibility
  config.showcursor = false;

  // Advanced browser settings
  if (options.main_world_eval) {
    config.allowMainWorld = options.main_world_eval;
  }

  config.forceScopeAccess = false;
  config.enableRemoteSubframes = false;
  config.disableTheming = false;
  config.memorysaver = false;

  return config;
}

/**
 * Create minimal user.js for Firefox-specific settings that are not part of Camoufox fingerprint config
 */
function createMinimalUserJs(
  profilePath: string,
  options: CamoufoxLaunchOptions,
): void {
  const preferences: string[] = [];

  // Basic privacy settings
  preferences.push('user_pref("privacy.resistFingerprinting", false);'); // Let Camoufox handle fingerprinting
  preferences.push('user_pref("privacy.trackingprotection.enabled", true);');

  // Disable telemetry and data collection
  preferences.push(
    'user_pref("datareporting.healthreport.uploadEnabled", false);',
  );
  preferences.push(
    'user_pref("datareporting.policy.dataSubmissionEnabled", false);',
  );
  preferences.push('user_pref("toolkit.telemetry.enabled", false);');
  preferences.push('user_pref("toolkit.telemetry.unified", false);');

  // Block options
  if (options.block_images) {
    preferences.push('user_pref("permissions.default.image", 2);');
  }

  if (options.block_webrtc) {
    preferences.push('user_pref("media.peerconnection.enabled", false);');
    preferences.push('user_pref("media.navigator.enabled", false);');
  }

  if (options.block_webgl) {
    preferences.push('user_pref("webgl.disabled", true);');
    preferences.push('user_pref("webgl.disable-extensions", true);');
  }

  // COOP settings
  if (options.disable_coop) {
    preferences.push(
      'user_pref("browser.tabs.remote.useCrossOriginOpenerPolicy", false);',
    );
  }

  // Proxy settings
  if (options.proxy) {
    if (typeof options.proxy === "string") {
      // Parse proxy URL
      try {
        const proxyUrl = new URL(options.proxy);
        const port =
          parseInt(proxyUrl.port) ||
          (proxyUrl.protocol === "https:" ? 443 : 80);

        if (proxyUrl.protocol.startsWith("socks")) {
          preferences.push('user_pref("network.proxy.type", 1);');
          preferences.push(
            `user_pref("network.proxy.socks", "${proxyUrl.hostname}");`,
          );
          preferences.push(`user_pref("network.proxy.socks_port", ${port});`);
          if (proxyUrl.protocol === "socks5:") {
            preferences.push('user_pref("network.proxy.socks_version", 5);');
          } else {
            preferences.push('user_pref("network.proxy.socks_version", 4);');
          }
        } else {
          preferences.push('user_pref("network.proxy.type", 1);');
          preferences.push(
            `user_pref("network.proxy.http", "${proxyUrl.hostname}");`,
          );
          preferences.push(`user_pref("network.proxy.http_port", ${port});`);
          preferences.push(
            `user_pref("network.proxy.ssl", "${proxyUrl.hostname}");`,
          );
          preferences.push(`user_pref("network.proxy.ssl_port", ${port});`);
        }

        if (proxyUrl.username && proxyUrl.password) {
          // Note: Basic auth for proxies is handled differently in modern Firefox
          preferences.push(
            'user_pref("network.proxy.allow_hijacking_localhost", true);',
          );
        }
      } catch (error) {
        console.error(`Invalid proxy URL: ${options.proxy}`);
      }
    }
  }

  // Custom Firefox preferences
  if (options.firefox_user_prefs) {
    for (const [key, value] of Object.entries(options.firefox_user_prefs)) {
      if (typeof value === "string") {
        preferences.push(`user_pref("${key}", "${value}");`);
      } else if (typeof value === "boolean") {
        preferences.push(`user_pref("${key}", ${value});`);
      } else if (typeof value === "number") {
        preferences.push(`user_pref("${key}", ${value});`);
      }
    }
  }

  // Cache settings
  if (options.enable_cache === false) {
    preferences.push('user_pref("browser.cache.disk.enable", false);');
    preferences.push('user_pref("browser.cache.memory.enable", false);');
  }

  // Write user.js file only if we have preferences to set
  if (preferences.length > 0) {
    const userJsPath = path.join(profilePath, "user.js");
    fs.writeFileSync(userJsPath, preferences.join("\n"));
  }
}

/**
 * Set Camoufox configuration via environment variables
 */
function setCamoufoxConfigEnv(config: any, env: NodeJS.ProcessEnv): void {
  const configJson = JSON.stringify(config);
  const chunkSize = os.platform() === "win32" ? 2047 : 32767;

  // Clear any existing CAMOU_CONFIG_* variables
  for (const key in env) {
    if (key.startsWith("CAMOU_CONFIG_")) {
      delete env[key];
    }
  }

  // Split config into chunks
  const chunks: string[] = [];
  for (let i = 0; i < configJson.length; i += chunkSize) {
    chunks.push(configJson.slice(i, i + chunkSize));
  }

  // Set environment variables (start from index 1 as expected by Camoufox)
  for (let i = 0; i < chunks.length; i++) {
    env[`CAMOU_CONFIG_${i + 1}`] = chunks[i];
  }
}

/**
 * Launch Camoufox browser with specified options
 */
export async function launchCamoufox(
  executablePath: string,
  profilePath: string,
  options: CamoufoxLaunchOptions = {},
  url?: string,
): Promise<CamoufoxConfig> {
  const id = generateCamoufoxId();

  // Ensure profile directory exists
  if (!fs.existsSync(profilePath)) {
    fs.mkdirSync(profilePath, { recursive: true });
  }

  // Create Camoufox configuration
  const camoufoxConfig = createCamoufoxConfig(options);

  // Create minimal user.js for Firefox-specific settings (proxy, blocking, etc.)
  createMinimalUserJs(profilePath, options);

  // Build command line arguments
  const args = buildCamoufoxArgs(options, profilePath, url);

  // Prepare environment variables
  const env: NodeJS.ProcessEnv = {
    ...process.env,
  };

  // Add custom environment variables from options, converting values to strings
  if (options.env) {
    for (const [key, value] of Object.entries(options.env)) {
      if (value !== undefined) {
        env[key] = String(value);
      }
    }
  }

  // Set Camoufox configuration via environment variables
  setCamoufoxConfigEnv(camoufoxConfig, env);

  if (options.debug) {
    console.log(
      "Camoufox configuration:",
      JSON.stringify(camoufoxConfig, null, 2),
    );
    console.log(
      "Environment variables set:",
      Object.keys(env).filter((key) => key.startsWith("CAMOU_CONFIG_")),
    );
  }

  // Handle virtual display
  if (options.virtual_display) {
    env.DISPLAY = options.virtual_display;
  }

  // Launch the process
  const child = spawn(executablePath, args, {
    env: env as NodeJS.ProcessEnv,
    detached: true,
    stdio: options.debug ? "inherit" : "ignore",
  });

  if (!child.pid) {
    throw new Error("Failed to launch Camoufox process");
  }

  const config: CamoufoxConfig = {
    id,
    pid: child.pid,
    executablePath,
    profilePath,
    url,
    options,
  };

  // Save configuration
  saveCamoufoxConfig(config);

  // Handle process exit
  child.on("exit", (code, signal) => {
    console.log(
      `Camoufox process ${child.pid} exited with code ${code}, signal ${signal}`,
    );
    deleteCamoufoxConfig(id);
  });

  child.on("error", (error) => {
    console.error(`Camoufox process error: ${error}`);
    deleteCamoufoxConfig(id);
  });

  // Detach the child process so it can continue running independently
  child.unref();

  return config;
}

/**
 * Stop a Camoufox process by ID
 */
export async function stopCamoufox(id: string): Promise<boolean> {
  const config = activeCamoufoxProcesses.get(id) || loadCamoufoxConfig(id);

  if (!config || !config.pid) {
    return false;
  }

  try {
    if (isProcessRunning(config.pid)) {
      process.kill(config.pid, "SIGTERM");

      // Wait a moment for graceful shutdown
      await new Promise((resolve) => setTimeout(resolve, 2000));

      // Force kill if still running
      if (isProcessRunning(config.pid)) {
        process.kill(config.pid, "SIGKILL");
      }
    }

    deleteCamoufoxConfig(id);
    return true;
  } catch (error) {
    console.error(`Failed to stop Camoufox process: ${error}`);
    return false;
  }
}

/**
 * List all Camoufox processes
 */
export function listCamoufoxProcesses(): any[] {
  loadAllCamoufoxConfigs();

  // Filter out dead processes
  const activeConfigs: any[] = [];

  for (const [id, config] of activeCamoufoxProcesses) {
    if (config.pid && isProcessRunning(config.pid)) {
      // Ensure we have the required fields, fall back to empty strings if missing
      const executablePath = config.executablePath || "";
      const profilePath = config.profilePath || "";

      // Return in snake_case format for Rust compatibility
      // Always include executable_path and profile_path, even if empty
      activeConfigs.push({
        id: config.id,
        pid: config.pid,
        executable_path: executablePath,
        profile_path: profilePath,
        url: config.url || null,
        options: config.options || {},
      });
    } else {
      // Clean up dead processes
      deleteCamoufoxConfig(id);
    }
  }

  return activeConfigs;
}

// Load existing configurations on module initialization
loadAllCamoufoxConfigs();
