"use client";

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import MultipleSelector, { type Option } from "@/components/multiple-selector";
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
  CamoufoxConfig,
  CamoufoxFingerprintConfig,
  CamoufoxOS,
} from "@/types";

interface SharedCamoufoxConfigFormProps {
  config: CamoufoxConfig;
  onConfigChange: (key: keyof CamoufoxConfig, value: unknown) => void;
  className?: string;
  isCreating?: boolean; // Flag to indicate if this is for creating a new profile
  forceAdvanced?: boolean; // Force advanced mode (for editing)
  readOnly?: boolean; // Flag to indicate if the form should be read-only
  browserType?: "camoufox" | "wayfern"; // Browser type to customize form options
  crossOsUnlocked?: boolean; // Allow selecting non-current OS (paid feature)
  limitedMode?: boolean; // Blur and disable advanced fields while keeping basic options accessible
}

// Determine if fingerprint editing should be disabled
const isFingerprintEditingDisabled = (config: CamoufoxConfig): boolean => {
  return config.randomize_fingerprint_on_launch === true;
};

// Detect the current operating system
const getCurrentOS = (): CamoufoxOS => {
  if (typeof navigator === "undefined") return "linux";
  const platform = navigator.platform.toLowerCase();
  if (platform.includes("win")) return "windows";
  if (platform.includes("mac")) return "macos";
  return "linux";
};

// OS display labels
const osLabels: Record<CamoufoxOS, string> = {
  windows: "Windows",
  macos: "macOS",
  linux: "Linux",
};

// Component for editing nested objects like webGl:parameters
interface ObjectEditorProps {
  value: Record<string, unknown> | undefined;
  onChange: (value: Record<string, unknown> | undefined) => void;
  title: string;
  readOnly?: boolean;
}

function ObjectEditor({
  value,
  onChange,
  title,
  readOnly = false,
}: ObjectEditorProps) {
  const [jsonString, setJsonString] = useState("");

  useEffect(() => {
    setJsonString(JSON.stringify(value || {}, null, 2));
  }, [value]);

  const handleChange = (newValue: string) => {
    if (readOnly) return;
    setJsonString(newValue);
    try {
      if (newValue.trim() === "" || newValue.trim() === "{}") {
        onChange(undefined); // Treat empty objects as undefined
        return;
      }
      const parsed = JSON.parse(newValue);
      if (
        typeof parsed === "object" &&
        parsed !== null &&
        Object.keys(parsed).length === 0
      ) {
        onChange(undefined);
        return;
      }
      onChange(parsed as Record<string, unknown>);
    } catch (err) {
      console.warn("Invalid JSON:", err);
    }
  };

  return (
    <div className="space-y-2">
      <Label>{title}</Label>
      <Textarea
        value={jsonString}
        onChange={(e) => handleChange(e.target.value)}
        placeholder={`Enter ${title} as JSON`}
        className="font-mono text-sm"
        rows={6}
        disabled={readOnly}
      />
    </div>
  );
}

export function SharedCamoufoxConfigForm({
  config,
  onConfigChange,
  className = "",
  isCreating = false,
  forceAdvanced = false,
  readOnly = false,
  browserType = "camoufox",
  crossOsUnlocked = false,
  limitedMode = false,
}: SharedCamoufoxConfigFormProps) {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState(
    forceAdvanced ? "manual" : "automatic",
  );
  const [fingerprintConfig, setFingerprintConfig] =
    useState<CamoufoxFingerprintConfig>({});
  const [currentOS] = useState<CamoufoxOS>(getCurrentOS);

  // Get selected OS (defaults to current OS)
  const selectedOS = config.os || currentOS;

  // Set screen resolution to user's screen size when creating a new profile
  useEffect(() => {
    if (isCreating && typeof window !== "undefined") {
      const screenWidth = window.screen.width;
      const screenHeight = window.screen.height;

      // Only set if not already configured
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

  // Parse fingerprint config when component mounts or config changes
  useEffect(() => {
    if (config.fingerprint) {
      try {
        const parsed = JSON.parse(
          config.fingerprint,
        ) as CamoufoxFingerprintConfig;
        setFingerprintConfig(parsed);
      } catch (error) {
        console.error("Failed to parse fingerprint config:", error);
        setFingerprintConfig({});
      }
    } else {
      // Initialize with empty config if no fingerprint is set
      setFingerprintConfig({});
    }
  }, [config.fingerprint]);

  // Update fingerprint config and serialize it
  const updateFingerprintConfig = (
    key: keyof CamoufoxFingerprintConfig,
    value: unknown,
  ) => {
    const newConfig = { ...fingerprintConfig };

    // Remove undefined values to keep the config clean
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

    // Validate that the config can be serialized to JSON
    try {
      const jsonString = JSON.stringify(newConfig);
      onConfigChange("fingerprint", jsonString);
    } catch (error) {
      console.error("Failed to serialize fingerprint config:", error);
      // Don't update if serialization fails
    }
  };

  // Determine if automatic location configuration is enabled
  const isAutoLocationEnabled = config.geoip !== false;

  // Handle automatic location configuration toggle
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
          onValueChange={(value: CamoufoxOS) => onConfigChange("os", value)}
          disabled={readOnly}
        >
          <SelectTrigger>
            <SelectValue placeholder={t("fingerprint.selectOSPlaceholder")} />
          </SelectTrigger>
          <SelectContent>
            {(["windows", "macos", "linux"] as CamoufoxOS[]).map((os) => {
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
                {t("fingerprint.advancedWarning")}
              </AlertDescription>
            </Alert>
          ))}

        <fieldset
          disabled={isEditingDisabled || limitedMode}
          className="space-y-6"
        >
          {/* Blocking Options - Only available for Camoufox */}
          {browserType === "camoufox" && (
            <div className="space-y-3">
              <Label>{t("fingerprint.blockingOptions")}</Label>
              <div className="space-y-2">
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="block-images"
                    checked={config.block_images || false}
                    onCheckedChange={(checked) =>
                      onConfigChange("block_images", checked)
                    }
                  />
                  <Label htmlFor="block-images">
                    {t("fingerprint.blockImages")}
                  </Label>
                </div>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="block-webrtc"
                    checked={config.block_webrtc || false}
                    onCheckedChange={(checked) =>
                      onConfigChange("block_webrtc", checked)
                    }
                  />
                  <Label htmlFor="block-webrtc">
                    {t("fingerprint.blockWebRTC")}
                  </Label>
                </div>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="block-webgl"
                    checked={config.block_webgl || false}
                    onCheckedChange={(checked) =>
                      onConfigChange("block_webgl", checked)
                    }
                  />
                  <Label htmlFor="block-webgl">
                    {t("fingerprint.blockWebGL")}
                  </Label>
                </div>
              </div>
            </div>
          )}

          {/* Navigator Properties */}
          <div className="space-y-3">
            <Label>{t("fingerprint.navigatorProperties")}</Label>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="user-agent">{t("fingerprint.userAgent")}</Label>
                <Input
                  id="user-agent"
                  value={fingerprintConfig["navigator.userAgent"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "navigator.userAgent",
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
                  value={fingerprintConfig["navigator.platform"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "navigator.platform",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., MacIntel, Win32"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="app-version">
                  {t("fingerprint.appVersion")}
                </Label>
                <Input
                  id="app-version"
                  value={fingerprintConfig["navigator.appVersion"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "navigator.appVersion",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., 5.0 (Macintosh)"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="oscpu">{t("fingerprint.osCpu")}</Label>
                <Input
                  id="oscpu"
                  value={fingerprintConfig["navigator.oscpu"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "navigator.oscpu",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., Intel Mac OS X 10.15"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="hardware-concurrency">
                  {t("fingerprint.hardwareConcurrency")}
                </Label>
                <Input
                  id="hardware-concurrency"
                  type="number"
                  value={
                    fingerprintConfig["navigator.hardwareConcurrency"] || ""
                  }
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "navigator.hardwareConcurrency",
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
                  value={fingerprintConfig["navigator.maxTouchPoints"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "navigator.maxTouchPoints",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 0"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="do-not-track">
                  {t("fingerprint.doNotTrack")}
                </Label>
                <Select
                  value={fingerprintConfig["navigator.doNotTrack"] || ""}
                  onValueChange={(value) =>
                    updateFingerprintConfig(
                      "navigator.doNotTrack",
                      value || undefined,
                    )
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
              <div className="space-y-2">
                <Label htmlFor="language">{t("fingerprint.language")}</Label>
                <Input
                  id="language"
                  value={fingerprintConfig["navigator.language"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "navigator.language",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., en-US"
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
                  value={fingerprintConfig["screen.width"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "screen.width",
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
                  value={fingerprintConfig["screen.height"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "screen.height",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 1080"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="avail-width">
                  {t("fingerprint.availableWidth")}
                </Label>
                <Input
                  id="avail-width"
                  type="number"
                  value={fingerprintConfig["screen.availWidth"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "screen.availWidth",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 1920"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="avail-height">
                  {t("fingerprint.availableHeight")}
                </Label>
                <Input
                  id="avail-height"
                  type="number"
                  value={fingerprintConfig["screen.availHeight"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "screen.availHeight",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 1055"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="color-depth">
                  {t("fingerprint.colorDepth")}
                </Label>
                <Input
                  id="color-depth"
                  type="number"
                  value={fingerprintConfig["screen.colorDepth"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "screen.colorDepth",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 30"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="pixel-depth">
                  {t("fingerprint.pixelDepth")}
                </Label>
                <Input
                  id="pixel-depth"
                  type="number"
                  value={fingerprintConfig["screen.pixelDepth"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "screen.pixelDepth",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 30"
                />
              </div>
            </div>
          </div>

          {/* Window Properties */}
          <div className="space-y-3">
            <Label>{t("fingerprint.windowProperties")}</Label>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label htmlFor="outer-width">
                  {t("fingerprint.outerWidth")}
                </Label>
                <Input
                  id="outer-width"
                  type="number"
                  value={fingerprintConfig["window.outerWidth"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "window.outerWidth",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 1512"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="outer-height">
                  {t("fingerprint.outerHeight")}
                </Label>
                <Input
                  id="outer-height"
                  type="number"
                  value={fingerprintConfig["window.outerHeight"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "window.outerHeight",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 886"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="inner-width">
                  {t("fingerprint.innerWidth")}
                </Label>
                <Input
                  id="inner-width"
                  type="number"
                  value={fingerprintConfig["window.innerWidth"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "window.innerWidth",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 1512"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="inner-height">
                  {t("fingerprint.innerHeight")}
                </Label>
                <Input
                  id="inner-height"
                  type="number"
                  value={fingerprintConfig["window.innerHeight"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "window.innerHeight",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 886"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="screen-x">{t("fingerprint.screenX")}</Label>
                <Input
                  id="screen-x"
                  type="number"
                  value={fingerprintConfig["window.screenX"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "window.screenX",
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
                  value={fingerprintConfig["window.screenY"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "window.screenY",
                      e.target.value ? parseInt(e.target.value, 10) : undefined,
                    )
                  }
                  placeholder="e.g., 0"
                />
              </div>
            </div>
          </div>

          {/* Geolocation */}
          <div className="space-y-3">
            <Label>{t("fingerprint.geolocation")}</Label>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label htmlFor="latitude">{t("fingerprint.latitude")}</Label>
                <Input
                  id="latitude"
                  type="number"
                  step="any"
                  value={fingerprintConfig["geolocation:latitude"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "geolocation:latitude",
                      e.target.value ? parseFloat(e.target.value) : undefined,
                    )
                  }
                  placeholder="e.g., 41.0019"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="longitude">{t("fingerprint.longitude")}</Label>
                <Input
                  id="longitude"
                  type="number"
                  step="any"
                  value={fingerprintConfig["geolocation:longitude"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "geolocation:longitude",
                      e.target.value ? parseFloat(e.target.value) : undefined,
                    )
                  }
                  placeholder="e.g., 28.9645"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="timezone">{t("fingerprint.timezone")}</Label>
                <Input
                  id="timezone"
                  type="text"
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
            </div>
          </div>

          {/* Locale */}
          <div className="space-y-3">
            <Label>{t("fingerprint.locale")}</Label>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label htmlFor="locale-language">
                  {t("fingerprint.language")}
                </Label>
                <Input
                  id="locale-language"
                  value={fingerprintConfig["locale:language"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "locale:language",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., tr"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="locale-region">{t("fingerprint.region")}</Label>
                <Input
                  id="locale-region"
                  value={fingerprintConfig["locale:region"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "locale:region",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., TR"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="locale-script">{t("fingerprint.script")}</Label>
                <Input
                  id="locale-script"
                  value={fingerprintConfig["locale:script"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "locale:script",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., Latn"
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
                  value={fingerprintConfig["webGl:vendor"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "webGl:vendor",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., Mesa"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="webgl-renderer">
                  {t("fingerprint.webglRenderer")}
                </Label>
                <Input
                  id="webgl-renderer"
                  value={fingerprintConfig["webGl:renderer"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "webGl:renderer",
                      e.target.value || undefined,
                    )
                  }
                  placeholder="e.g., llvmpipe, or similar"
                />
              </div>
            </div>
          </div>

          {/* WebGL Parameters */}
          <div className="space-y-3">
            <ObjectEditor
              value={
                (fingerprintConfig["webGl:parameters"] as Record<
                  string,
                  unknown
                >) || {}
              }
              onChange={(value) =>
                updateFingerprintConfig("webGl:parameters", value)
              }
              title={t("fingerprint.webglParameters")}
              readOnly={readOnly}
            />
          </div>

          {/* WebGL2 Parameters */}
          <div className="space-y-3">
            <ObjectEditor
              value={
                (fingerprintConfig["webGl2:parameters"] as Record<
                  string,
                  unknown
                >) || {}
              }
              onChange={(value) =>
                updateFingerprintConfig("webGl2:parameters", value)
              }
              title={t("fingerprint.webgl2Parameters")}
              readOnly={readOnly}
            />
          </div>

          {/* WebGL Shader Precision Formats */}
          <div className="space-y-3">
            <ObjectEditor
              value={
                (fingerprintConfig["webGl:shaderPrecisionFormats"] as Record<
                  string,
                  unknown
                >) || {}
              }
              onChange={(value) =>
                updateFingerprintConfig("webGl:shaderPrecisionFormats", value)
              }
              title={t("fingerprint.webglShaderPrecisionFormats")}
              readOnly={readOnly}
            />
          </div>

          {/* WebGL2 Shader Precision Formats */}
          <div className="space-y-3">
            <ObjectEditor
              value={
                (fingerprintConfig["webGl2:shaderPrecisionFormats"] as Record<
                  string,
                  unknown
                >) || {}
              }
              onChange={(value) =>
                updateFingerprintConfig("webGl2:shaderPrecisionFormats", value)
              }
              title={t("fingerprint.webgl2ShaderPrecisionFormats")}
              readOnly={readOnly}
            />
          </div>

          {/* Fonts */}
          <div className="space-y-3">
            <Label>{t("fingerprint.fonts")}</Label>
            <MultipleSelector
              value={(() => {
                // Handle fonts being either an array or a JSON string (Wayfern format)
                let fontsArray: string[] = [];
                if (fingerprintConfig.fonts) {
                  if (Array.isArray(fingerprintConfig.fonts)) {
                    fontsArray = fingerprintConfig.fonts;
                  } else if (typeof fingerprintConfig.fonts === "string") {
                    try {
                      const parsed = JSON.parse(fingerprintConfig.fonts);
                      if (Array.isArray(parsed)) {
                        fontsArray = parsed;
                      }
                    } catch {
                      // Invalid JSON, ignore
                    }
                  }
                }
                return fontsArray.map((font) => ({
                  label: font,
                  value: font,
                }));
              })()}
              onChange={(selected: Option[]) =>
                updateFingerprintConfig(
                  "fonts",
                  selected.map((s: Option) => s.value),
                )
              }
              placeholder="Add fonts..."
              creatable
            />
          </div>

          {/* Battery */}
          <div className="space-y-3">
            <Label>{t("fingerprint.battery")}</Label>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="battery-charging"
                    checked={fingerprintConfig["battery:charging"] || false}
                    onCheckedChange={(checked) =>
                      updateFingerprintConfig("battery:charging", checked)
                    }
                  />
                  <Label htmlFor="battery-charging">
                    {t("fingerprint.charging")}
                  </Label>
                </div>
              </div>
              <div className="space-y-2">
                <Label htmlFor="charging-time">
                  {t("fingerprint.chargingTime")}
                </Label>
                <Input
                  id="charging-time"
                  type="number"
                  step="any"
                  value={fingerprintConfig["battery:chargingTime"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "battery:chargingTime",
                      e.target.value ? parseFloat(e.target.value) : undefined,
                    )
                  }
                  placeholder="e.g., 0"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="discharging-time">
                  {t("fingerprint.dischargingTime")}
                </Label>
                <Input
                  id="discharging-time"
                  type="number"
                  step="any"
                  value={fingerprintConfig["battery:dischargingTime"] || ""}
                  onChange={(e) =>
                    updateFingerprintConfig(
                      "battery:dischargingTime",
                      e.target.value ? parseFloat(e.target.value) : undefined,
                    )
                  }
                  placeholder="e.g., 0"
                />
              </div>
            </div>
          </div>

          {/* Browser Behavior */}
          {/* <div className="space-y-3">
        <Label>Browser Behavior</Label>
        <div className="flex items-center space-x-2">
          <Checkbox
            id="allow-addon-new-tab"
            checked={fingerprintConfig.allowAddonNewTab}
            onCheckedChange={(checked) =>
              updateFingerprintConfig("allowAddonNewTab", checked)
            }
          />
          <Label htmlFor="allow-addon-new-tab">
            Allow browser addons to open new tabs automatically
          </Label>
        </div>
      </div> */}
        </fieldset>
        {limitedMode && (
          <>
            <div className="absolute inset-0 backdrop-blur-[6px] bg-background/30 z-[1]" />
            <div className="absolute inset-y-0 left-0 w-6 bg-gradient-to-r from-background to-transparent z-[2]" />
            <div className="absolute inset-y-0 right-0 w-6 bg-gradient-to-l from-background to-transparent z-[2]" />
            <div className="absolute inset-x-0 top-0 h-6 bg-gradient-to-b from-background to-transparent z-[2]" />
            <div className="absolute inset-x-0 bottom-0 h-6 bg-gradient-to-t from-background to-transparent z-[2]" />
            <div className="absolute inset-0 flex items-center justify-center z-[3]">
              <div className="flex items-center gap-2 rounded-md bg-background/80 px-3 py-1.5">
                <ProBadge />
                <span className="text-sm font-medium text-muted-foreground">
                  {t("fingerprint.proFeature")}
                </span>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );

  return (
    <div className={`space-y-6 ${className}`}>
      {forceAdvanced ? (
        // Advanced mode only (for editing)
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
                onValueChange={(value: CamoufoxOS) =>
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
                  {(["windows", "macos", "linux"] as CamoufoxOS[]).map((os) => {
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
                {t("fingerprint.generateRandomDescriptionAuto")}
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
                  <div className="absolute inset-0 backdrop-blur-[6px] bg-background/30 z-[1]" />
                  <div className="absolute inset-y-0 left-0 w-6 bg-gradient-to-r from-background to-transparent z-[2]" />
                  <div className="absolute inset-y-0 right-0 w-6 bg-gradient-to-l from-background to-transparent z-[2]" />
                  <div className="absolute inset-x-0 top-0 h-6 bg-gradient-to-b from-background to-transparent z-[2]" />
                  <div className="absolute inset-x-0 bottom-0 h-6 bg-gradient-to-t from-background to-transparent z-[2]" />
                  <div className="absolute inset-0 flex items-center justify-center z-[3]">
                    <div className="flex items-center gap-2 rounded-md bg-background/80 px-3 py-1.5">
                      <ProBadge />
                      <span className="text-sm font-medium text-muted-foreground">
                        {t("fingerprint.proFeature")}
                      </span>
                    </div>
                  </div>
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
