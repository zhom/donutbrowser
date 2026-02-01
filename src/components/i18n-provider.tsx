"use client";

import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { I18nextProvider } from "react-i18next";
import i18n, { getLanguageWithFallback, SUPPORTED_LANGUAGES } from "@/i18n";

interface AppSettings {
  language?: string | null;
  [key: string]: unknown;
}

interface I18nProviderProps {
  children: React.ReactNode;
}

export function I18nProvider({ children }: I18nProviderProps) {
  const [isReady, setIsReady] = useState(false);

  useEffect(() => {
    const initializeLanguage = async () => {
      try {
        const settings = await invoke<AppSettings>("get_app_settings");
        let language = settings.language;

        if (!language) {
          const systemLanguage = await invoke<string>("get_system_language");
          language = getLanguageWithFallback(systemLanguage);
        }

        if (
          language &&
          SUPPORTED_LANGUAGES.some((lang) => lang.code === language)
        ) {
          await i18n.changeLanguage(language);
        }
      } catch (error) {
        console.error("Failed to initialize language:", error);
      } finally {
        setIsReady(true);
      }
    };

    void initializeLanguage();
  }, []);

  if (!isReady) {
    return null;
  }

  return <I18nextProvider i18n={i18n}>{children}</I18nextProvider>;
}
