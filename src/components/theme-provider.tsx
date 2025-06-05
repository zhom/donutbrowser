"use client";

import { invoke } from "@tauri-apps/api/core";
import { ThemeProvider } from "next-themes";
import { useEffect, useState } from "react";

interface AppSettings {
  show_settings_on_startup: boolean;
  theme: string;
}

interface SystemTheme {
  theme: string;
}

interface CustomThemeProviderProps {
  children: React.ReactNode;
}

// Helper function to detect system dark mode preference
function getSystemTheme(): string {
  if (typeof window !== "undefined") {
    const isDarkMode = window.matchMedia(
      "(prefers-color-scheme: dark)",
    ).matches;
    return isDarkMode ? "dark" : "light";
  }
  return "light";
}

// Function to get native system theme (fallback to CSS media query)
async function getNativeSystemTheme(): Promise<string> {
  try {
    const systemTheme = await invoke<SystemTheme>("get_system_theme");
    if (systemTheme.theme === "dark" || systemTheme.theme === "light") {
      return systemTheme.theme;
    }
    // Fallback to CSS media query if native detection returns "unknown"
    return getSystemTheme();
  } catch (error) {
    console.warn(
      "Failed to get native system theme, falling back to CSS media query:",
      error,
    );
    // Fallback to CSS media query
    return getSystemTheme();
  }
}

export function CustomThemeProvider({ children }: CustomThemeProviderProps) {
  const [isLoading, setIsLoading] = useState(true);
  const [defaultTheme, setDefaultTheme] = useState<string>("system");
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    setMounted(true);
  }, []);

  useEffect(() => {
    const loadTheme = async () => {
      try {
        const settings = await invoke<AppSettings>("get_app_settings");
        setDefaultTheme(settings.theme);
      } catch (error) {
        console.error("Failed to load theme settings:", error);
        // For first-time users, detect system preference and apply it
        const systemTheme = await getNativeSystemTheme();
        console.log(
          "First-time user detected, applying system theme:",
          systemTheme,
        );

        // Save the detected theme as the default
        try {
          await invoke("save_app_settings", {
            settings: {
              show_settings_on_startup: true,
              theme: "system",
              auto_updates_enabled: true,
            },
          });
        } catch (saveError) {
          console.error("Failed to save initial theme settings:", saveError);
        }

        setDefaultTheme("system");
      } finally {
        setIsLoading(false);
      }
    };

    void loadTheme();
  }, []);

  // Monitor system theme changes when using "system" theme
  useEffect(() => {
    if (!mounted || defaultTheme !== "system") {
      return;
    }

    const checkSystemTheme = async () => {
      try {
        const currentSystemTheme = await getNativeSystemTheme();
        // Force re-evaluation by toggling the theme
        const html = document.documentElement;
        const currentClass = html.className;

        // Apply the system theme class
        if (currentSystemTheme === "dark") {
          if (!html.classList.contains("dark")) {
            html.classList.add("dark");
            html.classList.remove("light");
          }
        } else {
          if (
            !html.classList.contains("light") ||
            html.classList.contains("dark")
          ) {
            html.classList.add("light");
            html.classList.remove("dark");
          }
        }
      } catch (error) {
        console.warn("Failed to check system theme:", error);
      }
    };

    // Check system theme every 2 seconds when using system theme
    const intervalId = setInterval(() => void checkSystemTheme(), 2000);

    // Initial check
    void checkSystemTheme();

    return () => {
      clearInterval(intervalId);
    };
  }, [mounted, defaultTheme]);

  if (isLoading) {
    // Use a consistent loading screen that doesn't depend on system theme during SSR
    // This prevents hydration mismatch by ensuring server and client render the same initially
    let loadingBgColor = "bg-white";
    let spinnerColor = "border-gray-900";

    // Only apply system theme detection after component is mounted (client-side only)
    if (mounted) {
      // Use CSS media query for loading screen since async call would complicate this
      const systemTheme = getSystemTheme();
      loadingBgColor = systemTheme === "dark" ? "bg-gray-900" : "bg-white";
      spinnerColor =
        systemTheme === "dark" ? "border-white" : "border-gray-900";
    }

    return (
      <div
        className={`flex fixed inset-0 justify-center items-center ${loadingBgColor}`}
      >
        <div
          className={`w-8 h-8 rounded-full border-2 animate-spin ${spinnerColor} border-t-transparent`}
        />
      </div>
    );
  }

  return (
    <ThemeProvider
      attribute="class"
      defaultTheme={defaultTheme}
      enableSystem={true}
      disableTransitionOnChange={false}
    >
      {children}
    </ThemeProvider>
  );
}
