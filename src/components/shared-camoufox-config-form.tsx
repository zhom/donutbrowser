"use client";

import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { CamoufoxConfig } from "@/types";

const osOptions = [
  { value: "windows", label: "Windows" },
  { value: "macos", label: "macOS" },
  { value: "linux", label: "Linux" },
];

const timezoneOptions = [
  { value: "America/New_York", label: "America/New_York" },
  { value: "America/Los_Angeles", label: "America/Los_Angeles" },
  { value: "America/Chicago", label: "America/Chicago" },
  { value: "America/Denver", label: "America/Denver" },
  { value: "America/Phoenix", label: "America/Phoenix" },
  { value: "America/Toronto", label: "America/Toronto" },
  { value: "America/Vancouver", label: "America/Vancouver" },
  { value: "Europe/London", label: "Europe/London" },
  { value: "Europe/Paris", label: "Europe/Paris" },
  { value: "Europe/Berlin", label: "Europe/Berlin" },
  { value: "Europe/Rome", label: "Europe/Rome" },
  { value: "Europe/Madrid", label: "Europe/Madrid" },
  { value: "Europe/Amsterdam", label: "Europe/Amsterdam" },
  { value: "Europe/Zurich", label: "Europe/Zurich" },
  { value: "Europe/Vienna", label: "Europe/Vienna" },
  { value: "Europe/Warsaw", label: "Europe/Warsaw" },
  { value: "Europe/Prague", label: "Europe/Prague" },
  { value: "Europe/Stockholm", label: "Europe/Stockholm" },
  { value: "Europe/Copenhagen", label: "Europe/Copenhagen" },
  { value: "Europe/Helsinki", label: "Europe/Helsinki" },
  { value: "Europe/Oslo", label: "Europe/Oslo" },
  { value: "Europe/Brussels", label: "Europe/Brussels" },
  { value: "Europe/Dublin", label: "Europe/Dublin" },
  { value: "Europe/Lisbon", label: "Europe/Lisbon" },
  { value: "Europe/Athens", label: "Europe/Athens" },
  { value: "Europe/Budapest", label: "Europe/Budapest" },
  { value: "Europe/Bucharest", label: "Europe/Bucharest" },
  { value: "Europe/Sofia", label: "Europe/Sofia" },
  { value: "Europe/Kiev", label: "Europe/Kiev" },
  { value: "Europe/Moscow", label: "Europe/Moscow" },
  { value: "Asia/Tokyo", label: "Asia/Tokyo" },
  { value: "Asia/Seoul", label: "Asia/Seoul" },
  { value: "Asia/Shanghai", label: "Asia/Shanghai" },
  { value: "Asia/Hong_Kong", label: "Asia/Hong_Kong" },
  { value: "Asia/Singapore", label: "Asia/Singapore" },
  { value: "Asia/Bangkok", label: "Asia/Bangkok" },
  { value: "Asia/Jakarta", label: "Asia/Jakarta" },
  { value: "Asia/Manila", label: "Asia/Manila" },
  { value: "Asia/Kolkata", label: "Asia/Kolkata" },
  { value: "Asia/Dubai", label: "Asia/Dubai" },
  { value: "Asia/Riyadh", label: "Asia/Riyadh" },
  { value: "Asia/Tehran", label: "Asia/Tehran" },
  { value: "Asia/Jerusalem", label: "Asia/Jerusalem" },
  { value: "Asia/Istanbul", label: "Asia/Istanbul" },
  { value: "Australia/Sydney", label: "Australia/Sydney" },
  { value: "Australia/Melbourne", label: "Australia/Melbourne" },
  { value: "Australia/Brisbane", label: "Australia/Brisbane" },
  { value: "Australia/Perth", label: "Australia/Perth" },
  { value: "Australia/Adelaide", label: "Australia/Adelaide" },
  { value: "Pacific/Auckland", label: "Pacific/Auckland" },
  { value: "Pacific/Honolulu", label: "Pacific/Honolulu" },
  { value: "Africa/Cairo", label: "Africa/Cairo" },
  { value: "Africa/Johannesburg", label: "Africa/Johannesburg" },
  { value: "Africa/Lagos", label: "Africa/Lagos" },
  { value: "Africa/Nairobi", label: "Africa/Nairobi" },
  { value: "America/Sao_Paulo", label: "America/Sao_Paulo" },
  { value: "America/Buenos_Aires", label: "America/Buenos_Aires" },
  { value: "America/Lima", label: "America/Lima" },
  { value: "America/Bogota", label: "America/Bogota" },
  { value: "America/Santiago", label: "America/Santiago" },
  { value: "America/Caracas", label: "America/Caracas" },
  { value: "America/Mexico_City", label: "America/Mexico_City" },
];

const localeOptions = [
  { value: "en-US", label: "English (US)" },
  { value: "en-GB", label: "English (UK)" },
  { value: "en-CA", label: "English (Canada)" },
  { value: "en-AU", label: "English (Australia)" },
  { value: "fr-FR", label: "French (France)" },
  { value: "fr-CA", label: "French (Canada)" },
  { value: "de-DE", label: "German (Germany)" },
  { value: "de-AT", label: "German (Austria)" },
  { value: "de-CH", label: "German (Switzerland)" },
  { value: "es-ES", label: "Spanish (Spain)" },
  { value: "es-MX", label: "Spanish (Mexico)" },
  { value: "es-AR", label: "Spanish (Argentina)" },
  { value: "it-IT", label: "Italian (Italy)" },
  { value: "it-CH", label: "Italian (Switzerland)" },
  { value: "pt-BR", label: "Portuguese (Brazil)" },
  { value: "pt-PT", label: "Portuguese (Portugal)" },
  { value: "ru-RU", label: "Russian (Russia)" },
  { value: "zh-CN", label: "Chinese (Simplified)" },
  { value: "zh-TW", label: "Chinese (Traditional)" },
  { value: "ja-JP", label: "Japanese (Japan)" },
  { value: "ko-KR", label: "Korean (Korea)" },
  { value: "ar-SA", label: "Arabic (Saudi Arabia)" },
  { value: "ar-EG", label: "Arabic (Egypt)" },
  { value: "hi-IN", label: "Hindi (India)" },
  { value: "tr-TR", label: "Turkish (Turkey)" },
  { value: "pl-PL", label: "Polish (Poland)" },
  { value: "nl-NL", label: "Dutch (Netherlands)" },
  { value: "nl-BE", label: "Dutch (Belgium)" },
  { value: "sv-SE", label: "Swedish (Sweden)" },
  { value: "da-DK", label: "Danish (Denmark)" },
  { value: "no-NO", label: "Norwegian (Norway)" },
  { value: "fi-FI", label: "Finnish (Finland)" },
  { value: "he-IL", label: "Hebrew (Israel)" },
  { value: "th-TH", label: "Thai (Thailand)" },
  { value: "vi-VN", label: "Vietnamese (Vietnam)" },
  { value: "id-ID", label: "Indonesian (Indonesia)" },
  { value: "ms-MY", label: "Malay (Malaysia)" },
  { value: "uk-UA", label: "Ukrainian (Ukraine)" },
  { value: "cs-CZ", label: "Czech (Czech Republic)" },
  { value: "sk-SK", label: "Slovak (Slovakia)" },
  { value: "hu-HU", label: "Hungarian (Hungary)" },
  { value: "ro-RO", label: "Romanian (Romania)" },
  { value: "bg-BG", label: "Bulgarian (Bulgaria)" },
  { value: "hr-HR", label: "Croatian (Croatia)" },
  { value: "sr-RS", label: "Serbian (Serbia)" },
  { value: "sl-SI", label: "Slovenian (Slovenia)" },
  { value: "lt-LT", label: "Lithuanian (Lithuania)" },
  { value: "lv-LV", label: "Latvian (Latvia)" },
  { value: "et-EE", label: "Estonian (Estonia)" },
  { value: "el-GR", label: "Greek (Greece)" },
  { value: "ca-ES", label: "Catalan (Spain)" },
  { value: "eu-ES", label: "Basque (Spain)" },
  { value: "gl-ES", label: "Galician (Spain)" },
  { value: "is-IS", label: "Icelandic (Iceland)" },
  { value: "mt-MT", label: "Maltese (Malta)" },
];

const getCurrentOS = () => {
  if (typeof window !== "undefined") {
    const userAgent = window.navigator.userAgent;
    if (userAgent.includes("Win")) return "windows";
    if (userAgent.includes("Mac")) return "macos";
    if (userAgent.includes("Linux")) return "linux";
  }
  return "unknown";
};

interface SystemLocale {
  locale: string;
  language: string;
  country: string;
}

interface SystemTimezone {
  timezone: string;
  offset: string;
}

interface SharedCamoufoxConfigFormProps {
  config: CamoufoxConfig;
  onConfigChange: (key: keyof CamoufoxConfig, value: unknown) => void;
  className?: string;
}

export function SharedCamoufoxConfigForm({
  config,
  onConfigChange,
  className = "",
}: SharedCamoufoxConfigFormProps) {
  const [systemLocale, setSystemLocale] = useState<SystemLocale | null>(null);
  const [systemTimezone, setSystemTimezone] = useState<SystemTimezone | null>(
    null,
  );
  const [isLoadingSystemDefaults, setIsLoadingSystemDefaults] = useState(true);

  // Load system defaults on component mount
  useEffect(() => {
    const loadSystemDefaults = async () => {
      try {
        const [locale, timezone] = await Promise.all([
          invoke<SystemLocale>("get_system_locale"),
          invoke<SystemTimezone>("get_system_timezone"),
        ]);
        setSystemLocale(locale);
        setSystemTimezone(timezone);
      } catch (error) {
        console.error("Failed to load system defaults:", error);
        // Set fallback defaults
        setSystemLocale({
          locale: "en-US",
          language: "en",
          country: "US",
        });
        setSystemTimezone({
          timezone: "America/New_York",
          offset: "-05:00",
        });
      } finally {
        setIsLoadingSystemDefaults(false);
      }
    };

    loadSystemDefaults();
  }, []);

  // Get the selected OS for warning
  const selectedOS = config.os?.[0];
  const currentOS = getCurrentOS();
  const showOSWarning =
    selectedOS && selectedOS !== currentOS && currentOS !== "unknown";

  return (
    <div className={`space-y-6 ${className}`}>
      {/* OS Selection */}
      <div className="space-y-3">
        <Label>Operating System</Label>
        <Select
          value={config.os?.[0] || getCurrentOS()}
          onValueChange={(value) => onConfigChange("os", [value])}
        >
          <SelectTrigger>
            <SelectValue placeholder="Select OS" />
          </SelectTrigger>
          <SelectContent>
            {osOptions.map((os) => (
              <SelectItem key={os.value} value={os.value}>
                {os.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {showOSWarning && (
          <p className="text-sm text-yellow-600 dark:text-yellow-400">
            ⚠️ Selected OS ({selectedOS}) differs from your current OS (
            {currentOS}). This may affect fingerprinting effectiveness.
          </p>
        )}
      </div>

      {/* Privacy & Blocking */}
      <div className="space-y-3">
        <Label>Privacy & Blocking</Label>
        <div className="space-y-2">
          <div className="flex items-center space-x-2">
            <Checkbox
              id="block-images"
              checked={config.block_images || false}
              onCheckedChange={(checked) =>
                onConfigChange("block_images", checked)
              }
            />
            <Label htmlFor="block-images">Block Images</Label>
          </div>
          <div className="flex items-center space-x-2">
            <Checkbox
              id="block-webrtc"
              checked={config.block_webrtc || false}
              onCheckedChange={(checked) =>
                onConfigChange("block_webrtc", checked)
              }
            />
            <Label htmlFor="block-webrtc">Block WebRTC</Label>
          </div>
          <div className="flex items-center space-x-2">
            <Checkbox
              id="block-webgl"
              checked={config.block_webgl || false}
              onCheckedChange={(checked) =>
                onConfigChange("block_webgl", checked)
              }
            />
            <Label htmlFor="block-webgl">Block WebGL</Label>
          </div>
        </div>
      </div>

      {/* Geolocation */}
      <div className="space-y-3">
        <Label>Geolocation</Label>
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label htmlFor="country">Country</Label>
            <Input
              id="country"
              value={config.country || ""}
              onChange={(e) =>
                onConfigChange("country", e.target.value || undefined)
              }
              placeholder={
                systemLocale
                  ? `e.g., ${systemLocale.country}`
                  : "e.g., US, GB, DE"
              }
            />
          </div>
          <div className="space-y-2">
            <Label>Timezone</Label>
            <Select
              value={config.timezone || "auto"}
              onValueChange={(value) =>
                onConfigChange("timezone", value === "auto" ? undefined : value)
              }
            >
              <SelectTrigger>
                <SelectValue
                  placeholder={
                    isLoadingSystemDefaults ? "Loading..." : "Select timezone"
                  }
                />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="auto">
                  {isLoadingSystemDefaults
                    ? "Auto (loading...)"
                    : `Auto (${systemTimezone?.timezone || "UTC"})`}
                </SelectItem>
                {timezoneOptions.map((tz) => (
                  <SelectItem key={tz.value} value={tz.value}>
                    {tz.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label htmlFor="latitude">Latitude</Label>
            <Input
              id="latitude"
              type="number"
              step="any"
              value={config.latitude || ""}
              onChange={(e) =>
                onConfigChange(
                  "latitude",
                  e.target.value ? parseFloat(e.target.value) : undefined,
                )
              }
              placeholder="e.g., 40.7128"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="longitude">Longitude</Label>
            <Input
              id="longitude"
              type="number"
              step="any"
              value={config.longitude || ""}
              onChange={(e) =>
                onConfigChange(
                  "longitude",
                  e.target.value ? parseFloat(e.target.value) : undefined,
                )
              }
              placeholder="e.g., -74.0060"
            />
          </div>
        </div>
      </div>

      {/* Localization */}
      <div className="space-y-3">
        <Label>Locale</Label>
        <Select
          value={config.locale?.[0] || ""}
          onValueChange={(value) =>
            onConfigChange("locale", value ? [value] : undefined)
          }
        >
          <SelectTrigger>
            <SelectValue
              placeholder={
                isLoadingSystemDefaults
                  ? "Loading..."
                  : `Select locale (system: ${systemLocale?.locale || "unknown"})`
              }
            />
          </SelectTrigger>
          <SelectContent>
            {!isLoadingSystemDefaults && systemLocale && (
              <SelectItem value={systemLocale.locale}>
                {systemLocale.locale} (System Default)
              </SelectItem>
            )}
            {localeOptions.map((locale) => (
              <SelectItem key={locale.value} value={locale.value}>
                {locale.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {/* Screen Resolution */}
      <div className="space-y-3">
        <Label>Screen Resolution</Label>
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label htmlFor="screen-min-width">Min Width</Label>
            <Input
              id="screen-min-width"
              type="number"
              value={config.screen_min_width || ""}
              onChange={(e) =>
                onConfigChange(
                  "screen_min_width",
                  e.target.value ? parseInt(e.target.value) : undefined,
                )
              }
              placeholder="e.g., 1024"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="screen-max-width">Max Width</Label>
            <Input
              id="screen-max-width"
              type="number"
              value={config.screen_max_width || ""}
              onChange={(e) =>
                onConfigChange(
                  "screen_max_width",
                  e.target.value ? parseInt(e.target.value) : undefined,
                )
              }
              placeholder="e.g., 1920"
            />
          </div>
        </div>
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label htmlFor="screen-min-height">Min Height</Label>
            <Input
              id="screen-min-height"
              type="number"
              value={config.screen_min_height || ""}
              onChange={(e) =>
                onConfigChange(
                  "screen_min_height",
                  e.target.value ? parseInt(e.target.value) : undefined,
                )
              }
              placeholder="e.g., 768"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="screen-max-height">Max Height</Label>
            <Input
              id="screen-max-height"
              type="number"
              value={config.screen_max_height || ""}
              onChange={(e) =>
                onConfigChange(
                  "screen_max_height",
                  e.target.value ? parseInt(e.target.value) : undefined,
                )
              }
              placeholder="e.g., 1080"
            />
          </div>
        </div>
      </div>

      {/* Window Size */}
      <div className="space-y-3">
        <Label>Window Size</Label>
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label htmlFor="window-width">Width</Label>
            <Input
              id="window-width"
              type="number"
              value={config.window_width || ""}
              onChange={(e) =>
                onConfigChange(
                  "window_width",
                  e.target.value ? parseInt(e.target.value) : undefined,
                )
              }
              placeholder="e.g., 1366"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="window-height">Height</Label>
            <Input
              id="window-height"
              type="number"
              value={config.window_height || ""}
              onChange={(e) =>
                onConfigChange(
                  "window_height",
                  e.target.value ? parseInt(e.target.value) : undefined,
                )
              }
              placeholder="e.g., 768"
            />
          </div>
        </div>
      </div>

      {/* Advanced Options */}
      <div className="space-y-3">
        <Label>Advanced Options</Label>
        <div className="space-y-2">
          <div className="flex items-center space-x-2">
            <Checkbox
              id="enable-cache"
              checked={config.enable_cache || false}
              onCheckedChange={(checked) =>
                onConfigChange("enable_cache", checked)
              }
            />
            <Label htmlFor="enable-cache">Enable Browser Cache</Label>
          </div>
          <div className="flex items-center space-x-2">
            <Checkbox
              id="main-world-eval"
              checked={config.main_world_eval || false}
              onCheckedChange={(checked) =>
                onConfigChange("main_world_eval", checked)
              }
            />
            <Label htmlFor="main-world-eval">
              Enable Main World Evaluation
            </Label>
          </div>
        </div>
      </div>

      {/* WebGL Settings */}
      <div className="space-y-3">
        <Label>WebGL Settings</Label>
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label htmlFor="webgl-vendor">WebGL Vendor</Label>
            <Input
              id="webgl-vendor"
              value={config.webgl_vendor || ""}
              onChange={(e) =>
                onConfigChange("webgl_vendor", e.target.value || undefined)
              }
              placeholder="e.g., Intel Inc."
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="webgl-renderer">WebGL Renderer</Label>
            <Input
              id="webgl-renderer"
              value={config.webgl_renderer || ""}
              onChange={(e) =>
                onConfigChange("webgl_renderer", e.target.value || undefined)
              }
              placeholder="e.g., Intel HD Graphics"
            />
          </div>
        </div>
      </div>
    </div>
  );
}
