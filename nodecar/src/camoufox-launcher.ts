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
 * Create user.js file with Camoufox preferences
 */
function createUserJs(
  profilePath: string,
  options: CamoufoxLaunchOptions,
): void {
  const preferences: string[] = [];

  // Anti-detect preferences
  preferences.push('user_pref("privacy.resistFingerprinting", true);');
  preferences.push(
    'user_pref("privacy.resistFingerprinting.letterboxing", true);',
  );
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

  // Locale settings
  if (options.locale) {
    const localeStr = Array.isArray(options.locale)
      ? options.locale[0]
      : options.locale;
    preferences.push(`user_pref("intl.locale.requested", "${localeStr}");`);
    preferences.push(`user_pref("general.useragent.locale", "${localeStr}");`);
  }

  // Timezone
  if (options.timezone) {
    preferences.push(
      `user_pref("privacy.resistFingerprinting.timezone", "${options.timezone}");`,
    );
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

  // Geolocation
  if (options.geolocation) {
    preferences.push('user_pref("geo.enabled", true);');
    preferences.push(
      `user_pref("geo.wifi.uri", "data:application/json,{\\"location\\": {\\"lat\\": ${options.geolocation.latitude}, \\"lng\\": ${options.geolocation.longitude}}, \\"accuracy\\": ${options.geolocation.accuracy || 100}}");`,
    );
  } else {
    preferences.push('user_pref("geo.enabled", false);');
  }

  // Write user.js file
  const userJsPath = path.join(profilePath, "user.js");
  fs.writeFileSync(userJsPath, preferences.join("\n"));
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

  // Create user.js with preferences
  createUserJs(profilePath, options);

  // Build command line arguments
  const args = buildCamoufoxArgs(options, profilePath, url);

  // Prepare environment variables
  const env = {
    ...process.env,
    ...options.env,
  };

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
      // Return in snake_case format for Rust compatibility
      activeConfigs.push({
        id: config.id,
        pid: config.pid,
        executable_path: config.executablePath,
        profile_path: config.profilePath,
        url: config.url,
        options: config.options,
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
