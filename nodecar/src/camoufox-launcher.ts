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
  os?: "windows" | "macos" | "linux" | ("windows" | "macos" | "linux")[];

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
  exclude_addons?: "UBO"[];

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

  // Custom options - these may not be directly supported by camoufox-js
  timezone?: string;
  country?: string;
  geolocation?: {
    latitude: number;
    longitude: number;
    accuracy?: number;
  };

  // Add i_know_what_im_doing to match camoufox-js
  i_know_what_im_doing?: boolean;

  // Allow any additional properties that camoufox-js might accept
  [key: string]: any;
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

  try {
    // Use camoufox-js launchOptions to generate proper configuration
    // const { launchOptions } = require("camoufox-js");
    // const launchConfig = await launchOptions({
    //   ...options,
    //   executable_path: executablePath,
    //   // Enable debug if requested
    //   debug: options.debug || false,
    //   // Set i_know_what_im_doing to true to bypass warnings since we're controlling this
    //   i_know_what_im_doing: true,
    // });
    //
    const launchConfig: any = {};

    if (options.debug) {
      console.log(
        "Generated launch config:",
        JSON.stringify(launchConfig, null, 2),
      );
    }

    // Extract the command line args and environment from the launch config
    const args = [
      "-profile",
      profilePath,
      "-no-remote",
      ...(launchConfig.args || []),
    ];

    // Add URL if provided
    if (url) {
      args.push(url);
    }

    // Use the environment variables and other config from camoufox-js
    const env: NodeJS.ProcessEnv = {
      ...process.env,
      ...(launchConfig.env || {}),
    };

    if (options.debug) {
      console.log("Launch args:", args);
      console.log(
        "Environment variables set:",
        Object.keys(env).filter(
          (key) => key.startsWith("CAMOU_") || key.startsWith("DISPLAY"),
        ),
      );
    }

    // Use the executable path from the launch config if available
    const finalExecutablePath = launchConfig.executablePath || executablePath;

    // Write Firefox user preferences to user.js if provided
    if (
      launchConfig.firefoxUserPrefs &&
      Object.keys(launchConfig.firefoxUserPrefs).length > 0
    ) {
      const userJsPath = path.join(profilePath, "user.js");
      const preferences: string[] = [];

      for (const [key, value] of Object.entries(
        launchConfig.firefoxUserPrefs,
      )) {
        if (typeof value === "string") {
          preferences.push(`user_pref("${key}", "${value}");`);
        } else if (typeof value === "boolean") {
          preferences.push(`user_pref("${key}", ${value});`);
        } else if (typeof value === "number") {
          preferences.push(`user_pref("${key}", ${value});`);
        }
      }

      if (preferences.length > 0) {
        fs.writeFileSync(userJsPath, preferences.join("\n"));
      }
    }

    // Launch the process
    const child = spawn(finalExecutablePath, args, {
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
      executablePath: finalExecutablePath,
      profilePath,
      url,
      options,
    };

    // Save configuration
    saveCamoufoxConfig(config);

    // Handle process exit
    child.on("exit", (code, signal) => {
      if (options.debug) {
        console.log(
          `Camoufox process ${child.pid} exited with code ${code}, signal ${signal}`,
        );
      }
      deleteCamoufoxConfig(id);
    });

    child.on("error", (error) => {
      console.error(`Camoufox process error: ${error}`);
      deleteCamoufoxConfig(id);
    });

    // Detach the child process so it can continue running independently
    child.unref();

    return config;
  } catch (error) {
    console.error(`Failed to launch Camoufox: ${error}`);
    throw error;
  }
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
