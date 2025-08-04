import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

export function useBrowserSupport() {
  const [supportedBrowsers, setSupportedBrowsers] = useState<string[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const loadSupportedBrowsers = async () => {
      try {
        setIsLoading(true);
        setError(null);
        const browsers = await invoke<string[]>("get_supported_browsers");
        setSupportedBrowsers(browsers);
      } catch (err) {
        console.error("Failed to load supported browsers:", err);
        setError(
          err instanceof Error
            ? err.message
            : "Failed to load supported browsers",
        );
      } finally {
        setIsLoading(false);
      }
    };

    void loadSupportedBrowsers();
  }, []);

  const isBrowserSupported = (browser: string): boolean => {
    return supportedBrowsers.includes(browser);
  };

  const checkBrowserSupport = async (browser: string): Promise<boolean> => {
    try {
      return await invoke<boolean>("is_browser_supported_on_platform", {
        browserStr: browser,
      });
    } catch (err) {
      console.error(`Failed to check support for browser ${browser}:`, err);
      return false;
    }
  };

  return {
    supportedBrowsers,
    isLoading,
    error,
    isBrowserSupported,
    checkBrowserSupport,
  };
}
