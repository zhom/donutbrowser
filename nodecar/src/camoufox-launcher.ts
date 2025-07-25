import { launchOptions } from "camoufox-js";

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

/**
 * Generate Camoufox configuration using camoufox-js-lsd
 */
export async function generateCamoufoxConfig(
  options: CamoufoxLaunchOptions = {},
): Promise<any> {
  try {
    // Convert our options to camoufox-js-lsd format
    const camoufoxOptions: any = {};

    // Map our options to camoufox-js-lsd format
    if (options.os) camoufoxOptions.os = options.os;
    if (options.block_images !== undefined)
      camoufoxOptions.block_images = options.block_images;
    if (options.block_webrtc !== undefined)
      camoufoxOptions.block_webrtc = options.block_webrtc;
    if (options.block_webgl !== undefined)
      camoufoxOptions.block_webgl = options.block_webgl;
    if (options.disable_coop !== undefined)
      camoufoxOptions.disable_coop = options.disable_coop;
    if (options.geoip !== undefined) camoufoxOptions.geoip = options.geoip;
    if (options.humanize !== undefined)
      camoufoxOptions.humanize = options.humanize;
    if (options.locale) camoufoxOptions.locale = options.locale;
    if (options.addons) camoufoxOptions.addons = options.addons;
    if (options.fonts) camoufoxOptions.fonts = options.fonts;
    if (options.custom_fonts_only !== undefined)
      camoufoxOptions.custom_fonts_only = options.custom_fonts_only;
    if (options.exclude_addons)
      camoufoxOptions.exclude_addons = options.exclude_addons;
    if (options.screen) camoufoxOptions.screen = options.screen;
    if (options.window) camoufoxOptions.window = options.window;
    if (options.fingerprint) camoufoxOptions.fingerprint = options.fingerprint;
    if (options.ff_version !== undefined)
      camoufoxOptions.ff_version = options.ff_version;
    if (options.headless !== undefined)
      camoufoxOptions.headless = options.headless;
    if (options.main_world_eval !== undefined)
      camoufoxOptions.main_world_eval = options.main_world_eval;
    if (options.executable_path)
      camoufoxOptions.executable_path = options.executable_path;
    if (options.firefox_user_prefs)
      camoufoxOptions.firefox_user_prefs = options.firefox_user_prefs;
    if (options.proxy) camoufoxOptions.proxy = options.proxy;
    if (options.enable_cache !== undefined)
      camoufoxOptions.enable_cache = options.enable_cache;
    if (options.args) camoufoxOptions.args = options.args;
    if (options.env) camoufoxOptions.env = options.env;
    if (options.debug !== undefined) camoufoxOptions.debug = options.debug;
    if (options.virtual_display)
      camoufoxOptions.virtual_display = options.virtual_display;
    if (options.webgl_config)
      camoufoxOptions.webgl_config = options.webgl_config;

    // Handle custom options that might need mapping
    if (options.timezone) {
      // If timezone is provided directly, we can set it in the generated config
      // This will be handled after generation
    }
    if (options.country) {
      // Similar for country
    }
    if (options.geolocation) {
      // Handle geolocation coordinates
    }

    // Generate the configuration using camoufox-js-lsd
    const generatedConfig = await launchOptions(camoufoxOptions);

    // Apply any custom overrides
    if (options.timezone) {
      generatedConfig.env = generatedConfig.env || {};
      // The timezone will be handled in the CAMOU_CONFIG environment variable
    }

    return generatedConfig;
  } catch (error) {
    console.error(`Failed to generate Camoufox config: ${error}`);
    throw error;
  }
}
