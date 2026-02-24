"use client";

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ProBadge } from "@/components/ui/pro-badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import type {
  WayfernConfig,
  WayfernFingerprintConfig,
  WayfernOS,
} from "@/types";

interface WayfernConfigFormProps {
  config: WayfernConfig;
  onConfigChange: (key: keyof WayfernConfig, value: unknown) => void;
  className?: string;
  isCreating?: boolean;
  forceAdvanced?: boolean;
  readOnly?: boolean;
  crossOsUnlocked?: boolean;
  limitedMode?: boolean;
}

const isFingerprintEditingDisabled = (config: WayfernConfig): boolean => {
  return config.randomize_fingerprint_on_launch === true;
};

const getCurrentOS = (): WayfernOS => {
  if (typeof navigator === "undefined") return "linux";
  const platform = navigator.platform.toLowerCase();
  if (platform.includes("win")) return "windows";
  if (platform.includes("mac")) return "macos";
  return "linux";
};

const osLabels: Record<WayfernOS, string> = {
  windows: "Windows",
  macos: "macOS",
  linux: "Linux",
  android: "Android",
  ios: "iOS",
};

export function WayfernConfigForm({
  config,
  onConfigChange,
  className = "",
  isCreating = false,
  forceAdvanced = false,
  readOnly = false,
  crossOsUnlocked = false,
  limitedMode = false,
}: WayfernConfigFormProps) {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState(
    forceAdvanced ? "manual" : "automatic",
  );
  const [fingerprintConfig, setFingerprintConfig] =
    useState<WayfernFingerprintConfig>({});
  const [currentOS] = useState<WayfernOS>(getCurrentOS);

  const selectedOS = config.os || currentOS;

  useEffect(() => {
    if (isCreating && typeof window !== "undefined") {
      const screenWidth = window.screen.width;
      const screenHeight = window.screen.height;

      if (!config.screen_max_width) {
        onConfigChange("screen_max_width", screenWidth);
      }
      if (!config.screen_max_height) {
        onConfigChange("screen_max_height", screenHeight);
      }
    }
  }, [
    isCreating,
    config.screen_max_width,
    config.screen_max_height,
    onConfigChange,
  ]);

  useEffect(() => {
    if (config.fingerprint) {
      try {
        const parsed = JSON.parse(
          config.fingerprint,
        ) as WayfernFingerprintConfig;
        setFingerprintConfig(parsed);
      } catch (error) {
        console.error("Failed to parse fingerprint config:", error);
        setFingerprintConfig({});
      }
    } else {
      setFingerprintConfig({});
    }
  }, [config.fingerprint]);

  const updateFingerprintConfig = (
    key: keyof WayfernFingerprintConfig,
    value: unknown,
  ) => {
    const newConfig = { ...fingerprintConfig };

    if (
      value === undefined ||
      value === "" ||
      (Array.isArray(value) && value.length === 0)
    ) {
      delete newConfig[key];
    } else {
      (newConfig as Record<string, unknown>)[key] = value;
    }

    setFingerprintConfig(newConfig);

    try {
      const jsonString = JSON.stringify(newConfig);
      onConfigChange("fingerprint", jsonString);
    } catch (error) {
      console.error("Failed to serialize fingerprint config:", error);
    }
  };

  const isAutoLocationEnabled = config.geoip !== false;

  const handleAutoLocationToggle = (enabled: boolean) => {
    if (enabled) {
      onConfigChange("geoip", true);
    } else {
      onConfigChange("geoip", false);
    }
  };

  const isEditingDisabled = isFingerprintEditingDisabled(config) || readOnly;

  const renderAdvancedForm = () => (
    <div className="space-y-6">
      {/* Operating System Selection */}
      <div className="space-y-3">
        <Label>{t("fingerprint.osLabel")}</Label>
        <Select
          value={selectedOS}
          onValueChange={(value: WayfernOS) => onConfigChange("os", value)}
          disabled={readOnly}
        >
          <SelectTrigger>
            <SelectValue placeholder={t("fingerprint.selectOSPlaceholder")} />
          </SelectTrigger>
          <SelectContent>
            {(
              ["windows", "macos", "linux", "android", "ios"] as WayfernOS[]
            ).map((os) => {
              const isDisabled = os !== currentOS && !crossOsUnlocked;
              return (
                <SelectItem key={os} value={os} disabled={isDisabled}>
                  <span className="flex items-center gap-2">
                    {osLabels[os]}
                    {isDisabled && <ProBadge />}
                  </span>
                </SelectItem>
              );
            })}
          </SelectContent>
        </Select>
        {selectedOS !== currentOS && crossOsUnlocked && (
          <Alert className="mt-2">
            <AlertDescription>
              {t("fingerprint.crossOsWarning")}
            </AlertDescription>
          </Alert>
        )}
      </div>

      {/* Randomize Fingerprint Option */}
      <div className="space-y-3 p-4 border rounded-lg bg-muted/30">
        <div className="flex items-center space-x-2">
          <Checkbox
            id="randomize-fingerprint"
            checked={config.randomize_fingerprint_on_launch || false}
            onCheckedChange={(checked) =>
              onConfigChange("randomize_fingerprint_on_launch", checked)
            }
            disabled={readOnly}
          />
          <Label htmlFor="randomize-fingerprint" className="font-medium">
            {t("fingerprint.generateRandomOnLaunch")}
          </Label>
        </div>
        <p className="text-sm text-muted-foreground ml-6">
          {t("fingerprint.generateRandomDescription")}
        </p>
      </div>

      {/* Automatic Location Configuration */}
      <div className="space-y-3">
        <div className="flex items-center space-x-2">
          <Checkbox
            id="auto-location-advanced"
            checked={isAutoLocationEnabled}
            onCheckedChange={handleAutoLocationToggle}
            disabled={readOnly}
          />
          <Label htmlFor="auto-location-advanced">
            {t("fingerprint.autoLocationDescription")}
          </Label>
        </div>
      </div>

      <div
        className={
          limitedMode ? "relative overflow-hidden rounded-lg" : undefined
        }
      >
        {!limitedMode &&
          (isEditingDisabled ? (
            <Alert>
              <AlertDescription>
                {readOnly
                  ? t("fingerprint.editingDisabledRunning")
                  : t("fingerprint.editingDisabledRandomized")}
              </AlertDescription>
            </Alert>
          ) : (
            <Alert>
              <AlertDescription>
                {t("fingerprint.basicWarning")}
              </AlertDescription>
            </Alert>
          ))}

        <fieldset
          disabled={isEditingDisabled || limitedMode}
          className="space-y-6"
        >
          {/* User Agent and Platform */}
          <div className="space-y-3">
            <Label>{t("fingerprint.userAgentAndPlatform")}</Label>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2 col-span-2">
                <Label htmlFor="user-agent">{t("fingerprint.userAgent")}</Label>
                <Input
                  id="user-agent"
                  value={fingerprintConfig.userAgent || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "userAgent",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="Mozilla/5.0..."
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="platform">{t("fingerprint.platform")}</Label>
                <Input
                  id="platform"
                  value={fingerprintConfig.platform || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "platform",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., Win32, MacIntel, Linux x86_64"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="platform-version">
                  {t("fingerprint.platformVersion")}
                </Label>
                <Input
                  id="platform-version"
                  value={fingerprintConfig.platformVersion || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "platformVersion",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., 10.0.0"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="brand">{t("fingerprint.brand")}</Label>
                <Input
                  id="brand"
                  value={fingerprintConfig.brand || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "brand",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., Google Chrome"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="brand-version">
                  {t("fingerprint.brandVersion")}
                </Label>
                <Input
                  id="brand-version"
                  value={fingerprintConfig.brandVersion || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "brandVersion",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., 143"
                />
              </div>
            </div>
          </div>

          {/* Hardware Properties */}
          <div className="space-y-3">
            <Label>{t("fingerprint.hardwareProperties")}</Label>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label htmlFor="hardware-concurrency">
                  {t("fingerprint.hardwareConcurrency")}
                </Label>
                <Input
                  id="hardware-concurrency"
                  type="number"
                  value={fingerprintConfig.hardwareConcurrency || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "hardwareConcurrency",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 8"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="max-touch-points">
                  {t("fingerprint.maxTouchPoints")}
                </Label>
                <Input
                  id="max-touch-points"
                  type="number"
                  value={fingerprintConfig.maxTouchPoints || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "maxTouchPoints",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 0"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="device-memory">
                  {t("fingerprint.deviceMemory")}
                </Label>
                <Input
                  id="device-memory"
                  type="number"
                  value={fingerprintConfig.deviceMemory || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "deviceMemory",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 8"
                />
              </div>
            </div>
          </div>

          {/* Screen Properties */}
          <div className="space-y-3">
            <Label>{t("fingerprint.screenProperties")}</Label>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label htmlFor="screen-width">
                  {t("fingerprint.screenWidth")}
                </Label>
                <Input
                  id="screen-width"
                  type="number"
                  value={fingerprintConfig.screenWidth || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "screenWidth",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 1920"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="screen-height">
                  {t("fingerprint.screenHeight")}
                </Label>
                <Input
                  id="screen-height"
                  type="number"
                  value={fingerprintConfig.screenHeight || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "screenHeight",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 1080"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="device-pixel-ratio">
                  {t("fingerprint.devicePixelRatio")}
                </Label>
                <Input
                  id="device-pixel-ratio"
                  type="number"
                  step="0.1"
                  value={fingerprintConfig.devicePixelRatio || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "devicePixelRatio",
                      e.target.value ? parseFloat(e.target.value) : undefined,
                    )
                  }
                  placeholder="e.g., 1.0"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="screen-avail-width">
                  {t("fingerprint.availableWidth")}
                </Label>
                <Input
                  id="screen-avail-width"
                  type="number"
                  value={fingerprintConfig.screenAvailWidth || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "screenAvailWidth",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 1920"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="screen-avail-height">
                  {t("fingerprint.availableHeight")}
                </Label>
                <Input
                  id="screen-avail-height"
                  type="number"
                  value={fingerprintConfig.screenAvailHeight || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "screenAvailHeight",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 1040"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="screen-color-depth">
                  {t("fingerprint.colorDepth")}
                </Label>
                <Input
                  id="screen-color-depth"
                  type="number"
                  value={fingerprintConfig.screenColorDepth || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "screenColorDepth",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 24"
                />
              </div>
            </div>
          </div>

          {/* Window Properties */}
          <div className="space-y-3">
            <Label>{t("fingerprint.windowProperties")}</Label>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label htmlFor="window-outer-width">
                  {t("fingerprint.outerWidth")}
                </Label>
                <Input
                  id="window-outer-width"
                  type="number"
                  value={fingerprintConfig.windowOuterWidth || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "windowOuterWidth",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 1920"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="window-outer-height">
                  {t("fingerprint.outerHeight")}
                </Label>
                <Input
                  id="window-outer-height"
                  type="number"
                  value={fingerprintConfig.windowOuterHeight || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "windowOuterHeight",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 1040"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="window-inner-width">
                  {t("fingerprint.innerWidth")}
                </Label>
                <Input
                  id="window-inner-width"
                  type="number"
                  value={fingerprintConfig.windowInnerWidth || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "windowInnerWidth",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 1920"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="window-inner-height">
                  {t("fingerprint.innerHeight")}
                </Label>
                <Input
                  id="window-inner-height"
                  type="number"
                  value={fingerprintConfig.windowInnerHeight || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "windowInnerHeight",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 940"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="screen-x">{t("fingerprint.screenX")}</Label>
                <Input
                  id="screen-x"
                  type="number"
                  value={fingerprintConfig.screenX || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "screenX",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 0"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="screen-y">{t("fingerprint.screenY")}</Label>
                <Input
                  id="screen-y"
                  type="number"
                  value={fingerprintConfig.screenY || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "screenY",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 0"
                />
              </div>
            </div>
          </div>

          {/* Language & Locale */}
          <div className="space-y-3">
            <Label>{t("fingerprint.languageAndLocale")}</Label>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="language">
                  {t("fingerprint.primaryLanguage")}
                </Label>
                <Input
                  id="language"
                  value={fingerprintConfig.language || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "language",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., en-US"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="languages">{t("fingerprint.languages")}</Label>
                <Input
                  id="languages"
                  value={
                    Array.isArray(fingerprintConfig.languages)
                      ? JSON.stringify(fingerprintConfig.languages)
                      : ""
                  }
                  onChange={(e) => {
                    if (!e.target.value) {
                      updateFingerprintConfig("languages", undefined);
                      return;
                    }
                    try {
                      const parsed = JSON.parse(e.target.value);
                      if (Array.isArray(parsed)) {
                        updateFingerprintConfig("languages", parsed);
                      }
                    } catch {
                      // Invalid JSON, keep current value
                    }
                  }}
                  placeholder='["en-US", "en"]'
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="do-not-track">
                  {t("fingerprint.doNotTrack")}
                </Label>
                <Select
                  value={fingerprintConfig.doNotTrack || ""}
                  onValueChange={(value) =>
                    updateFingerprintConfig("doNotTrack", value || undefined)
                  }
                >
                  <SelectTrigger>
                    <SelectValue
                      placeholder={t("fingerprint.selectDntPlaceholder")}
                    />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="0">
                      {t("fingerprint.dntAllowed")}
                    </SelectItem>
                    <SelectItem value="1">
                      {t("fingerprint.dntNotAllowed")}
                    </SelectItem>
                    <SelectItem value="unspecified">
                      {t("fingerprint.dntUnspecified")}
                    </SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
          </div>

          {/* Timezone and Geolocation */}
          <div className="space-y-3">
            <Label>{t("fingerprint.timezoneAndGeolocation")}</Label>
            <p className="text-sm text-muted-foreground">
              {t("fingerprint.timezoneGeolocationDescription")}
            </p>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label htmlFor="timezone">
                  {t("fingerprint.timezoneIana")}
                </Label>
                <Input
                  id="timezone"
                  value={fingerprintConfig.timezone || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "timezone",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., America/New_York"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="timezone-offset">
                  {t("fingerprint.timezoneOffset")}
                </Label>
                <Input
                  id="timezone-offset"
                  type="number"
                  value={fingerprintConfig.timezoneOffset ?? ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "timezoneOffset",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 300 for EST (UTC-5)"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="latitude">{t("fingerprint.latitude")}</Label>
                <Input
                  id="latitude"
                  type="number"
                  step="any"
                  value={fingerprintConfig.latitude || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "latitude",
                      e.target.value ? parseFloat(e.target.value) : undefined,
                    )
                  }
                  placeholder="e.g., 40.7128"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="longitude">{t("fingerprint.longitude")}</Label>
                <Input
                  id="longitude"
                  type="number"
                  step="any"
                  value={fingerprintConfig.longitude || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "longitude",
                      e.target.value ? parseFloat(e.target.value) : undefined,
                    )
                  }
                  placeholder="e.g., -74.0060"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="accuracy">{t("fingerprint.accuracy")}</Label>
                <Input
                  id="accuracy"
                  type="number"
                  value={fingerprintConfig.accuracy || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "accuracy",
                      e.target.value ? parseFloat(e.target.value) : undefined,
                    )
                  }
                  placeholder="e.g., 100"
                />
              </div>
            </div>
          </div>

          {/* WebGL Properties */}
          <div className="space-y-3">
            <Label>{t("fingerprint.webglProperties")}</Label>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="webgl-vendor">
                  {t("fingerprint.webglVendor")}
                </Label>
                <Input
                  id="webgl-vendor"
                  value={fingerprintConfig.webglVendor || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "webglVendor",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., Intel"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="webgl-renderer">
                  {t("fingerprint.webglRenderer")}
                </Label>
                <Input
                  id="webgl-renderer"
                  value={fingerprintConfig.webglRenderer || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "webglRenderer",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., Intel(R) HD Graphics"
                />
              </div>
            </div>
          </div>

          {/* WebGL Parameters (JSON) */}
          <div className="space-y-3">
            <Label>{t("fingerprint.webglParametersJson")}</Label>
            <Textarea
              value={fingerprintConfig.webglParameters || ""}
              onChange={(e) =>
                updateFingerprintConfig(
                  "webglParameters",
                  e.target.value || undefined,
                )
              }
              placeholder='{"7936": "Intel", "7937": "Intel(R) HD Graphics"}'
              className="font-mono text-sm"
              rows={4}
            />
          </div>

          {/* Canvas Noise Seed */}
          <div className="space-y-3">
            <Label>{t("fingerprint.canvasFingerprint")}</Label>
            <div className="space-y-2">
              <Label htmlFor="canvas-noise-seed">
                {t("fingerprint.canvasNoiseSeed")}
              </Label>
              <Input
                id="canvas-noise-seed"
                value={fingerprintConfig.canvasNoiseSeed || ""}
                onChange={(e) =>
                  updateFingerprintConfig(
                    "canvasNoiseSeed",
                    e.target.value || undefined,
                  )
                }
                placeholder="Enter a seed string for canvas fingerprint"
              />
              <p className="text-sm text-muted-foreground">
                {t("fingerprint.canvasNoiseSeedDescription")}
              </p>
            </div>
          </div>

          {/* Fonts (JSON) */}
          <div className="space-y-3">
            <Label>{t("fingerprint.fontsJson")}</Label>
            <Textarea
              value={fingerprintConfig.fonts || ""}
              onChange={(e) =>
                updateFingerprintConfig("fonts", e.target.value || undefined)
              }
              placeholder='["Arial", "Verdana", "Times New Roman"]'
              className="font-mono text-sm"
              rows={3}
            />
          </div>

          {/* Audio */}
          <div className="space-y-3">
            <Label>{t("fingerprint.audioProperties")}</Label>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="audio-sample-rate">
                  {t("fingerprint.sampleRate")}
                </Label>
                <Input
                  id="audio-sample-rate"
                  type="number"
                  value={fingerprintConfig.audioSampleRate || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "audioSampleRate",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 48000"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="audio-max-channel-count">
                  {t("fingerprint.maxChannelCount")}
                </Label>
                <Input
                  id="audio-max-channel-count"
                  type="number"
                  value={fingerprintConfig.audioMaxChannelCount || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "audioMaxChannelCount",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 2"
                />
              </div>
            </div>
          </div>

          {/* Battery */}
          <div className="space-y-3">
            <Label>{t("fingerprint.battery")}</Label>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="battery-charging"
                    checked={fingerprintConfig.batteryCharging || false}
                    onCheckedChange={(checked) =>
                      updateFingerprintConfig(
                        "batteryCharging",
                        checked || undefined,
                      )
                    }
                  />
                  <Label htmlFor="battery-charging">
                    {t("fingerprint.charging")}
                  </Label>
                </div>
              </div>
              <div className="space-y-2">
                <Label htmlFor="battery-level">
                  {t("fingerprint.batteryLevel")}
                </Label>
                <Input
                  id="battery-level"
                  type="number"
                  step="0.01"
                  min="0"
                  max="1"
                  value={fingerprintConfig.batteryLevel || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "batteryLevel",
                      e.target.value ? parseFloat(e.target.value) : undefined,
                    )
                  }
                  placeholder="e.g., 0.85"
                />
              </div>
            </div>
          </div>

          {/* Vendor Info */}
          <div className="space-y-3">
            <Label>{t("fingerprint.vendorInfo")}</Label>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label htmlFor="vendor">{t("fingerprint.vendor")}</Label>
                <Input
                  id="vendor"
                  value={fingerprintConfig.vendor || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "vendor",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., Google Inc."
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="vendor-sub">{t("fingerprint.vendorSub")}</Label>
                <Input
                  id="vendor-sub"
                  value={fingerprintConfig.vendorSub || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "vendorSub",
                      e.target.value || undefined,
                    )
                  }
                  placeholder=""
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="product-sub">
                  {t("fingerprint.productSub")}
                </Label>
                <Input
                  id="product-sub"
                  value={fingerprintConfig.productSub || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "productSub",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., 20030107"
                />
              </div>
            </div>
          </div>
        </fieldset>
        {limitedMode && (
          <>
            <div className="absolute inset-0 backdrop-blur-[6px] bg-background/30" />
            <div className="absolute inset-0 flex items-center justify-center z-[2]">
              <div className="flex items-center gap-2">
                <ProBadge />
                <span className="text-sm font-medium text-muted-foreground">
                  {t("fingerprint.proFeature")}
                </span>
              </div>
            </div>
            <div className="absolute inset-y-0 left-0 w-6 bg-gradient-to-r from-background to-transparent z-[1]" />
            <div className="absolute inset-y-0 right-0 w-6 bg-gradient-to-l from-background to-transparent z-[1]" />
            <div className="absolute inset-x-0 bottom-0 h-6 bg-gradient-to-t from-background to-transparent z-[1]" />
          </>
        )}
      </div>
    </div>
  );

  return (
    <div className={`space-y-6 ${className}`}>
      {forceAdvanced ? (
        renderAdvancedForm()
      ) : (
        <Tabs
          value={activeTab}
          onValueChange={readOnly ? undefined : setActiveTab}
          className="w-full"
        >
          <TabsList className="grid grid-cols-2 w-full">
            <TabsTrigger value="automatic" disabled={readOnly}>
              {t("fingerprint.automatic")}
            </TabsTrigger>
            <TabsTrigger value="manual" disabled={readOnly}>
              {t("fingerprint.manual")}
            </TabsTrigger>
          </TabsList>

          <TabsContent value="automatic" className="space-y-6">
            {/* Operating System Selection */}
            <div className="mt-4 space-y-3">
              <Label>{t("fingerprint.osLabel")}</Label>
              <Select
                value={selectedOS}
                onValueChange={(value: WayfernOS) =>
                  onConfigChange("os", value)
                }
                disabled={readOnly}
              >
                <SelectTrigger>
                  <SelectValue
                    placeholder={t("fingerprint.selectOSPlaceholder")}
                  />
                </SelectTrigger>
                <SelectContent>
                  {(
                    [
                      "windows",
                      "macos",
                      "linux",
                      "android",
                      "ios",
                    ] as WayfernOS[]
                  ).map((os) => {
                    const isDisabled = os !== currentOS && !crossOsUnlocked;
                    return (
                      <SelectItem key={os} value={os} disabled={isDisabled}>
                        <span className="flex items-center gap-2">
                          {osLabels[os]}
                          {isDisabled && <ProBadge />}
                        </span>
                      </SelectItem>
                    );
                  })}
                </SelectContent>
              </Select>
              {selectedOS !== currentOS && crossOsUnlocked && (
                <Alert className="mt-2">
                  <AlertDescription>
                    {t("fingerprint.crossOsLimitations")}
                  </AlertDescription>
                </Alert>
              )}
            </div>

            {/* Randomize Fingerprint Option */}
            <div className="space-y-3 p-4 border rounded-lg bg-muted/30">
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="randomize-fingerprint-auto"
                  checked={config.randomize_fingerprint_on_launch || false}
                  onCheckedChange={(checked) =>
                    onConfigChange("randomize_fingerprint_on_launch", checked)
                  }
                  disabled={readOnly}
                />
                <Label
                  htmlFor="randomize-fingerprint-auto"
                  className="font-medium"
                >
                  {t("fingerprint.generateRandomOnLaunch")}
                </Label>
              </div>
              <p className="text-sm text-muted-foreground ml-6">
                {t("fingerprint.generateRandomDescription")}
              </p>
            </div>

            {/* Automatic Location Configuration */}
            <div className="space-y-3">
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="auto-location"
                  checked={isAutoLocationEnabled}
                  onCheckedChange={handleAutoLocationToggle}
                  disabled={isEditingDisabled}
                />
                <Label htmlFor="auto-location">
                  {t("fingerprint.autoLocationDescription")}
                </Label>
              </div>
            </div>

            {/* Screen Resolution */}
            <div
              className={
                limitedMode ? "relative overflow-hidden rounded-lg" : undefined
              }
            >
              <fieldset
                disabled={isEditingDisabled || limitedMode}
                className="space-y-3"
              >
                <Label>{t("fingerprint.screenResolution")}</Label>
                <div className="grid grid-cols-2 gap-4">
                  <div className="space-y-2">
                    <Label htmlFor="screen-max-width">
                      {t("fingerprint.maxWidth")}
                    </Label>
                    <Input
                      id="screen-max-width"
                      type="number"
                      value={config.screen_max_width || ""}
                      onChange={(e) =>
                        onConfigChange(
                          "screen_max_width",
                          e.target.value
                            ? parseInt(e.target.value, 10)
                            : undefined,
                        )
                      }
                      placeholder="e.g., 1920"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="screen-max-height">
                      {t("fingerprint.maxHeight")}
                    </Label>
                    <Input
                      id="screen-max-height"
                      type="number"
                      value={config.screen_max_height || ""}
                      onChange={(e) =>
                        onConfigChange(
                          "screen_max_height",
                          e.target.value
                            ? parseInt(e.target.value, 10)
                            : undefined,
                        )
                      }
                      placeholder="e.g., 1080"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="screen-min-width">
                      {t("fingerprint.minWidth")}
                    </Label>
                    <Input
                      id="screen-min-width"
                      type="number"
                      value={config.screen_min_width || ""}
                      onChange={(e) =>
                        onConfigChange(
                          "screen_min_width",
                          e.target.value
                            ? parseInt(e.target.value, 10)
                            : undefined,
                        )
                      }
                      placeholder="e.g., 800"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="screen-min-height">
                      {t("fingerprint.minHeight")}
                    </Label>
                    <Input
                      id="screen-min-height"
                      type="number"
                      value={config.screen_min_height || ""}
                      onChange={(e) =>
                        onConfigChange(
                          "screen_min_height",
                          e.target.value
                            ? parseInt(e.target.value, 10)
                            : undefined,
                        )
                      }
                      placeholder="e.g., 600"
                    />
                  </div>
                </div>
              </fieldset>
              {limitedMode && (
                <>
                  <div className="absolute inset-0 backdrop-blur-[6px] bg-background/30" />
                  <div className="absolute inset-0 flex items-center justify-center z-[2]">
                    <div className="flex items-center gap-2">
                      <ProBadge />
                      <span className="text-sm font-medium text-muted-foreground">
                        {t("fingerprint.proFeature")}
                      </span>
                    </div>
                  </div>
                  <div className="absolute inset-y-0 left-0 w-6 bg-gradient-to-r from-background to-transparent z-[1]" />
                  <div className="absolute inset-y-0 right-0 w-6 bg-gradient-to-l from-background to-transparent z-[1]" />
                  <div className="absolute inset-x-0 bottom-0 h-6 bg-gradient-to-t from-background to-transparent z-[1]" />
                </>
              )}
            </div>
          </TabsContent>

          <TabsContent value="manual" className="space-y-6">
            {renderAdvancedForm()}
          </TabsContent>
        </Tabs>
      )}
    </div>
  );
}
