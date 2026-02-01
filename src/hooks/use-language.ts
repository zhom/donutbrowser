import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  getLanguageWithFallback,
  SUPPORTED_LANGUAGES,
  type SupportedLanguage,
} from "@/i18n";

interface AppSettings {
  language?: string | null;
  [key: string]: unknown;
}

export function useLanguage() {
  const { i18n } = useTranslation();
  const [isLoading, setIsLoading] = useState(true);
  const [currentLanguage, setCurrentLanguage] = useState<string>("en");

  const loadLanguage = useCallback(async () => {
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
        setCurrentLanguage(language);
      }
    } catch (error) {
      console.error("Failed to load language setting:", error);
    } finally {
      setIsLoading(false);
    }
  }, [i18n]);

  const changeLanguage = useCallback(
    async (language: SupportedLanguage | null) => {
      try {
        const settings = await invoke<AppSettings>("get_app_settings");
        const updatedSettings = {
          ...settings,
          language,
        };
        await invoke("save_app_settings", { settings: updatedSettings });

        if (language) {
          await i18n.changeLanguage(language);
          setCurrentLanguage(language);
        } else {
          const systemLanguage = await invoke<string>("get_system_language");
          const fallbackLanguage = getLanguageWithFallback(systemLanguage);
          await i18n.changeLanguage(fallbackLanguage);
          setCurrentLanguage(fallbackLanguage);
        }
      } catch (error) {
        console.error("Failed to change language:", error);
        throw error;
      }
    },
    [i18n],
  );

  useEffect(() => {
    void loadLanguage();
  }, [loadLanguage]);

  return {
    currentLanguage,
    changeLanguage,
    isLoading,
    supportedLanguages: SUPPORTED_LANGUAGES,
  };
}
