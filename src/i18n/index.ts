import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./locales/en.json";
import es from "./locales/es.json";
import fr from "./locales/fr.json";
import ja from "./locales/ja.json";
import pt from "./locales/pt.json";
import ru from "./locales/ru.json";
import zh from "./locales/zh.json";

export const SUPPORTED_LANGUAGES = [
  { code: "en", name: "English", nativeName: "English" },
  { code: "es", name: "Spanish", nativeName: "Español" },
  { code: "pt", name: "Portuguese", nativeName: "Português" },
  { code: "fr", name: "French", nativeName: "Français" },
  { code: "zh", name: "Chinese", nativeName: "中文" },
  { code: "ja", name: "Japanese", nativeName: "日本語" },
  { code: "ru", name: "Russian", nativeName: "Русский" },
] as const;

export type SupportedLanguage = (typeof SUPPORTED_LANGUAGES)[number]["code"];

export const LANGUAGE_FALLBACKS: Record<string, string[]> = {
  uk: ["ru", "en"],
  be: ["ru", "en"],
  "zh-TW": ["zh", "en"],
  "zh-CN": ["zh", "en"],
  "zh-HK": ["zh", "en"],
  "pt-BR": ["pt", "en"],
  "pt-PT": ["pt", "en"],
  "es-MX": ["es", "en"],
  "es-AR": ["es", "en"],
  "es-ES": ["es", "en"],
  "fr-CA": ["fr", "en"],
  "fr-FR": ["fr", "en"],
};

export function getLanguageWithFallback(systemLocale: string): string {
  const baseLanguage = systemLocale.split(/[-_]/)[0].toLowerCase();

  if (SUPPORTED_LANGUAGES.some((lang) => lang.code === baseLanguage)) {
    return baseLanguage;
  }

  if (LANGUAGE_FALLBACKS[systemLocale]) {
    return LANGUAGE_FALLBACKS[systemLocale][0];
  }

  if (LANGUAGE_FALLBACKS[baseLanguage]) {
    return LANGUAGE_FALLBACKS[baseLanguage][0];
  }

  return "en";
}

const resources = {
  en: { translation: en },
  es: { translation: es },
  pt: { translation: pt },
  fr: { translation: fr },
  zh: { translation: zh },
  ja: { translation: ja },
  ru: { translation: ru },
};

i18n.use(initReactI18next).init({
  resources,
  lng: "en",
  fallbackLng: "en",
  interpolation: {
    escapeValue: false,
  },
  react: {
    useSuspense: false,
  },
});

export default i18n;
