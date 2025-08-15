"use client";

import { ThemeProvider } from "next-themes";
import { useEffect, useState } from "react";

interface AppSettings {
  set_as_default_browser: boolean;
  theme: string;
  custom_theme?: Record<string, string>;
}

interface CustomThemeProviderProps {
  children: React.ReactNode;
}

export function CustomThemeProvider({ children }: CustomThemeProviderProps) {
  const [isLoading, setIsLoading] = useState(true);
  const [defaultTheme, setDefaultTheme] = useState<string>("system");
  const [_mounted, setMounted] = useState(false);

  useEffect(() => {
    setMounted(true);
  }, []);

  useEffect(() => {
    const loadTheme = async () => {
      try {
        // Lazy import to avoid pulling Tauri API on SSR
        const { invoke } = await import("@tauri-apps/api/core");
        const settings = await invoke<AppSettings>("get_app_settings");
        const themeValue = settings?.theme ?? "system";

        if (
          themeValue === "light" ||
          themeValue === "dark" ||
          themeValue === "system"
        ) {
          setDefaultTheme(themeValue);
        } else if (themeValue === "custom") {
          setDefaultTheme("light");
          if (
            settings.custom_theme &&
            Object.keys(settings.custom_theme).length > 0
          ) {
            try {
              const root = document.documentElement;
              // Apply with !important to override CSS defaults
              Object.entries(settings.custom_theme).forEach(([k, v]) => {
                root.style.setProperty(k, v, "important");
              });
            } catch (error) {
              console.warn("Failed to apply custom theme variables:", error);
            }
          }
        } else {
          setDefaultTheme("system");
        }
      } catch (error) {
        // Failed to load settings; fall back to system (handled by next-themes)
        console.warn(
          "Failed to load theme settings; defaulting to system:",
          error,
        );
        setDefaultTheme("system");
      } finally {
        setIsLoading(false);
      }
    };

    void loadTheme();
  }, []);

  // Additional effect to ensure custom theme is applied after mount
  useEffect(() => {
    if (!isLoading && _mounted) {
      const reapplyCustomTheme = async () => {
        try {
          const { invoke } = await import("@tauri-apps/api/core");
          const settings = await invoke<AppSettings>("get_app_settings");

          if (settings?.theme === "custom" && settings.custom_theme) {
            const root = document.documentElement;
            Object.entries(settings.custom_theme).forEach(([k, v]) => {
              root.style.setProperty(k, v, "important");
            });
          }
        } catch (error) {
          console.warn("Failed to reapply custom theme:", error);
        }
      };

      // Apply after a short delay to ensure CSS has loaded
      setTimeout(reapplyCustomTheme, 100);
    }
  }, [isLoading, _mounted]);

  if (isLoading) {
    // Keep UI simple during initial settings load to avoid flicker
    return null;
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
