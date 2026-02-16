/**
 * Browser utility functions
 * Centralized helpers for browser name mapping, icons, etc.
 */

import { FaChrome, FaExclamationTriangle, FaFirefox } from "react-icons/fa";

/**
 * Map internal browser names to display names
 */
export function getBrowserDisplayName(browserType: string): string {
  const browserNames: Record<string, string> = {
    firefox: "Firefox",
    "firefox-developer": "Firefox Developer Edition",
    zen: "Zen Browser",
    brave: "Brave",
    chromium: "Chromium",
    camoufox: "Firefox (Camoufox)",
    wayfern: "Chromium (Wayfern)",
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
    case "camoufox":
      return FaFirefox; // Firefox-based anti-detect browser
    case "wayfern":
      return FaChrome; // Chromium-based anti-detect browser
    default:
      // All other browsers get a warning icon
      return FaExclamationTriangle;
  }
}

export const getCurrentOS = () => {
  if (typeof window !== "undefined") {
    const userAgent = window.navigator.userAgent;
    if (userAgent.includes("Win")) return "windows";
    if (userAgent.includes("Mac")) return "macos";
    if (userAgent.includes("Linux")) return "linux";
  }
  return "unknown";
};

export function isCrossOsProfile(profile: { host_os?: string }): boolean {
  if (!profile.host_os) return false;
  return profile.host_os !== getCurrentOS();
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
