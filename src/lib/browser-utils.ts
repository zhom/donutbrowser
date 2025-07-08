/**
 * Browser utility functions
 * Centralized helpers for browser name mapping, icons, etc.
 */

import { FaChrome, FaFirefox, FaShieldAlt } from "react-icons/fa";
import { SiBrave, SiMullvad, SiTorbrowser } from "react-icons/si";
import { ZenBrowser } from "@/components/icons/zen-browser";

/**
 * Map internal browser names to display names
 */
export function getBrowserDisplayName(browserType: string): string {
  const browserNames: Record<string, string> = {
    firefox: "Firefox",
    "firefox-developer": "Firefox Developer Edition",
    "mullvad-browser": "Mullvad Browser",
    zen: "Zen Browser",
    brave: "Brave",
    chromium: "Chromium",
    "tor-browser": "Tor Browser",
    camoufox: "Anti-Detect",
  };

  return browserNames[browserType] || browserType;
}

/**
 * Get the appropriate icon component for a browser type
 */
export function getBrowserIcon(browserType: string) {
  switch (browserType) {
    case "mullvad-browser":
      return SiMullvad;
    case "chromium":
      return FaChrome;
    case "brave":
      return SiBrave;
    case "firefox":
    case "firefox-developer":
      return FaFirefox;
    case "zen":
      return ZenBrowser;
    case "tor-browser":
      return SiTorbrowser;
    case "camoufox":
      return FaShieldAlt;
    default:
      return null;
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
