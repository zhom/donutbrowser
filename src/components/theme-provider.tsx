"use client";

import { invoke } from "@tauri-apps/api/core";
import { ThemeProvider } from "next-themes";
import { useEffect, useState } from "react";

interface AppSettings {
  show_settings_on_startup: boolean;
  theme: string;
}

interface CustomThemeProviderProps {
  children: React.ReactNode;
}

// Helper function to detect system dark mode preference
function getSystemTheme(): string {
  if (typeof window !== "undefined") {
    const isDarkMode = window.matchMedia(
      "(prefers-color-scheme: dark)"
    ).matches;
    return isDarkMode ? "dark" : "light";
  }
  return "light";
}

export function CustomThemeProvider({ children }: CustomThemeProviderProps) {
  const [isLoading, setIsLoading] = useState(true);
  const [defaultTheme, setDefaultTheme] = useState<string>("system");

  useEffect(() => {
    const loadTheme = async () => {
      try {
        const settings = await invoke<AppSettings>("get_app_settings");
        setDefaultTheme(settings.theme);
      } catch (error) {
        console.error("Failed to load theme settings:", error);
        // For first-time users, detect system preference and apply it
        const systemTheme = getSystemTheme();
        console.log(
          "First-time user detected, applying system theme:",
          systemTheme
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

  if (isLoading) {
    // Detect system theme to show appropriate loading screen
    const systemTheme = getSystemTheme();
    const loadingBgColor = systemTheme === "dark" ? "bg-gray-900" : "bg-white";
    const spinnerColor =
      systemTheme === "dark" ? "border-white" : "border-gray-900";

    return (
      <div
        className={`fixed inset-0 ${loadingBgColor} flex items-center justify-center`}
      >
        <div
          className={`animate-spin rounded-full h-8 w-8 border-2 ${spinnerColor} border-t-transparent`}
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
