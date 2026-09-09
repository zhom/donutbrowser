import { LuGlobe } from "react-icons/lu";
import { SiBrave, SiOpera, SiVivaldi } from "react-icons/si";
/**
 * Browser utility functions
 * Centralized helpers for browser name mapping, icons, etc.
 */

import {
  FaChrome,
  FaEdge,
  FaExclamationTriangle,
  FaFire,
  FaFirefox,
  FaSafari,
} from "react-icons/fa";
import { LuLock } from "react-icons/lu";
import { getCurrentOS } from "@/lib/platform";

export { getCurrentOS } from "@/lib/platform";

/**
 * Map internal browser names to display names
 */
export function getBrowserDisplayName(browserType: string): string {
  const browserNames: Record<string, string> = {
    wayfern: "Wayfern",
  };

  return browserNames[browserType] || browserType;
}

/**
 * Get the appropriate icon component for a browser type
 * Anti-detect browsers get their base browser icons
 * Other browsers get a warning icon to indicate they're not anti-detect
 */
export function getBrowserIcon(browserType: string) {
  switch (browserType) {
    case "wayfern":
      return FaChrome;
    default:
      return FaExclamationTriangle;
  }
}

/** Source browsers are identities, not warnings about the destination runtime. */
export function getSourceBrowserIcon(browserType: string) {
  switch (browserType.toLowerCase()) {
    case "chrome":
    case "chrome-beta":
    case "chrome-dev":
    case "chrome-canary":
    case "chromium":
    case "wayfern":
      return FaChrome;
    case "edge":
    case "msedge":
      return FaEdge;
    case "brave":
      return SiBrave;
    case "vivaldi":
      return SiVivaldi;
    case "opera":
    case "opera-gx":
      return SiOpera;
    case "firefox":
      return FaFirefox;
    case "safari":
      return FaSafari;
    default:
      return LuGlobe;
  }
}

export function getProfileIcon(profile: {
  browser: string;
  ephemeral?: boolean;
  password_protected?: boolean;
}) {
  // `password_protected` and `ephemeral` are mutually exclusive (the backend
  // rejects setting a password on an ephemeral profile), so the order here
  // doesn't matter — checking lock first only matters if the invariant is
  // ever violated, in which case showing the lock is the safer default.
  if (profile.password_protected) return LuLock;
  if (profile.ephemeral) return FaFire;
  return getBrowserIcon(profile.browser);
}

export function isCrossOsProfile(profile: {
  host_os?: string;
  wayfern_config?: { os?: string };
}): boolean {
  const profileOs = profile.host_os || profile.wayfern_config?.os;
  if (!profileOs) return false;
  return profileOs !== getCurrentOS();
}

export function getOSDisplayName(os: string): string {
  switch (os) {
    case "macos":
      return "macOS";
    case "windows":
      return "Windows";
    case "linux":
      return "Linux";
    default:
      return os;
  }
}
