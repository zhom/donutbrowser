"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";
import { applyThemeColors, clearThemeColors } from "@/lib/themes";

interface AppSettings {
  set_as_default_browser: boolean;
  theme: string;
  custom_theme?: Record<string, string>;
}

interface ThemeContextValue {
  theme: string;
  setTheme: (theme: string) => void;
}

const ThemeContext = createContext<ThemeContextValue>({
  theme: "system",
  setTheme: () => {},
});

export function useTheme() {
  return useContext(ThemeContext);
}

interface CustomThemeProviderProps {
  children: React.ReactNode;
}

function resolveSystemTheme(): "light" | "dark" {
  if (typeof window === "undefined") return "dark";
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

function applyClassToHtml(theme: string) {
  const resolved = theme === "system" ? resolveSystemTheme() : theme;
  const root = document.documentElement;
  root.classList.remove("light", "dark");
  root.classList.add(resolved);
}

export function CustomThemeProvider({ children }: CustomThemeProviderProps) {
  const [isLoading, setIsLoading] = useState(true);
  const [theme, setThemeState] = useState("system");

  const setTheme = useCallback((newTheme: string) => {
    setThemeState(newTheme);
    if (newTheme === "custom") {
      applyClassToHtml("dark");
    } else {
      applyClassToHtml(newTheme);
    }
  }, []);

  // Load initial theme from Tauri settings
  useEffect(() => {
    const loadTheme = async () => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const settings = await invoke<AppSettings>("get_app_settings");
        const themeValue = settings?.theme ?? "system";

        if (themeValue === "custom") {
          setThemeState("custom");
          applyClassToHtml("dark");
          if (
            settings.custom_theme &&
            Object.keys(settings.custom_theme).length > 0
          ) {
            try {
              applyThemeColors(settings.custom_theme);
            } catch (error) {
              console.warn("Failed to apply custom theme variables:", error);
            }
          }
        } else if (
          themeValue === "light" ||
          themeValue === "dark" ||
          themeValue === "system"
        ) {
          setThemeState(themeValue);
          applyClassToHtml(themeValue);
        } else {
          applyClassToHtml("system");
        }
      } catch (error) {
        console.warn(
          "Failed to load theme settings; defaulting to system:",
          error,
        );
        applyClassToHtml("system");
      } finally {
        setIsLoading(false);
      }
    };

    void loadTheme();
  }, []);

  // Re-apply custom theme after mount
  useEffect(() => {
    if (!isLoading && theme === "custom") {
      const reapplyCustomTheme = async () => {
        try {
          const { invoke } = await import("@tauri-apps/api/core");
          const settings = await invoke<AppSettings>("get_app_settings");
          if (settings?.theme === "custom" && settings.custom_theme) {
            applyThemeColors(settings.custom_theme);
          }
        } catch (error) {
          console.warn("Failed to reapply custom theme:", error);
        }
      };
      setTimeout(() => {
        void reapplyCustomTheme();
      }, 100);
    } else if (!isLoading) {
      clearThemeColors();
    }
  }, [isLoading, theme]);

  // Listen for system theme changes when in "system" mode
  useEffect(() => {
    if (theme !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = () => applyClassToHtml("system");
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, [theme]);

  const value = useMemo(() => ({ theme, setTheme }), [theme, setTheme]);

  if (isLoading) {
    return null;
  }

  return (
    <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
  );
}
