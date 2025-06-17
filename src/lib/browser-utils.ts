/**
 * Browser utility functions
 * Centralized helpers for browser name mapping, icons, etc.
 */

import { ZenBrowser } from "@/components/icons/zen-browser";
import { FaChrome, FaFirefox } from "react-icons/fa";
import { SiBrave, SiMullvad, SiTorbrowser } from "react-icons/si";

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
    default:
      return null;
  }
}
