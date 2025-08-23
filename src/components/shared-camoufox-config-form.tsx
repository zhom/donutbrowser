"use client";

import { useEffect, useState } from "react";
import MultipleSelector, { type Option } from "@/components/multiple-selector";
import { Alert, AlertDescription } from "@/components/ui/alert";
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import type { CamoufoxConfig, CamoufoxFingerprintConfig } from "@/types";

interface SharedCamoufoxConfigFormProps {
  config: CamoufoxConfig;
  onConfigChange: (key: keyof CamoufoxConfig, value: unknown) => void;
  className?: string;
  isCreating?: boolean; // Flag to indicate if this is for creating a new profile
  forceAdvanced?: boolean; // Force advanced mode (for editing)
}

// Component for editing nested objects like webGl:parameters
interface ObjectEditorProps {
  value: Record<string, unknown> | undefined;
  onChange: (value: Record<string, unknown> | undefined) => void;
  title: string;
}

function ObjectEditor({ value, onChange, title }: ObjectEditorProps) {
  const [jsonString, setJsonString] = useState("");

  useEffect(() => {
    setJsonString(JSON.stringify(value || {}, null, 2));
  }, [value]);

  const handleChange = (newValue: string) => {
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
}: SharedCamoufoxConfigFormProps) {
  const [activeTab, setActiveTab] = useState(
    forceAdvanced ? "manual" : "automatic",
  );
  const [fingerprintConfig, setFingerprintConfig] =
    useState<CamoufoxFingerprintConfig>({});

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

  const renderAdvancedForm = () => (
    <div className="space-y-6">
      <Alert>
        <AlertDescription>
          ⚠️ Warning: Only edit these parameters if you know what you're doing.
          Incorrect values may break websites, make them detect you, and lead to
          hard-to-debug bugs.{" "}
        </AlertDescription>
      </Alert>

      {/* Blocking Options */}
      <div className="space-y-3">
        <Label>Blocking Options</Label>
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

      {/* Navigator Properties */}
      <div className="space-y-3">
        <Label>Navigator Properties</Label>
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label htmlFor="user-agent">User Agent</Label>
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
            <Label htmlFor="platform">Platform</Label>
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
            <Label htmlFor="app-version">App Version</Label>
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
            <Label htmlFor="oscpu">OS CPU</Label>
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
            <Label htmlFor="hardware-concurrency">Hardware Concurrency</Label>
            <Input
              id="hardware-concurrency"
              type="number"
              value={fingerprintConfig["navigator.hardwareConcurrency"] || ""}
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
            <Label htmlFor="max-touch-points">Max Touch Points</Label>
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
            <Label htmlFor="do-not-track">Do Not Track</Label>
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
                <SelectValue placeholder="Select DNT value" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="0">0 (tracking allowed)</SelectItem>
                <SelectItem value="1">1 (tracking not allowed)</SelectItem>
                <SelectItem value="unspecified">unspecified</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-2">
            <Label htmlFor="language">Language</Label>
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
        <Label>Screen Properties</Label>
        <div className="grid grid-cols-3 gap-4">
          <div className="space-y-2">
            <Label htmlFor="screen-width">Screen Width</Label>
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
            <Label htmlFor="screen-height">Screen Height</Label>
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
            <Label htmlFor="avail-width">Available Width</Label>
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
            <Label htmlFor="avail-height">Available Height</Label>
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
            <Label htmlFor="color-depth">Color Depth</Label>
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
            <Label htmlFor="pixel-depth">Pixel Depth</Label>
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
        <Label>Window Properties</Label>
        <div className="grid grid-cols-3 gap-4">
          <div className="space-y-2">
            <Label htmlFor="outer-width">Outer Width</Label>
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
            <Label htmlFor="outer-height">Outer Height</Label>
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
            <Label htmlFor="inner-width">Inner Width</Label>
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
            <Label htmlFor="inner-height">Inner Height</Label>
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
            <Label htmlFor="screen-x">Screen X</Label>
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
            <Label htmlFor="screen-y">Screen Y</Label>
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
        <Label>Geolocation</Label>
        <div className="grid grid-cols-3 gap-4">
          <div className="space-y-2">
            <Label htmlFor="latitude">Latitude</Label>
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
            <Label htmlFor="longitude">Longitude</Label>
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
            <Label htmlFor="timezone">Timezone</Label>
            <Input
              id="timezone"
              type="text"
              value={fingerprintConfig.timezone || ""}
              onChange={(e) =>
                updateFingerprintConfig("timezone", e.target.value || undefined)
              }
              placeholder="e.g., America/New_York"
            />
          </div>
        </div>
      </div>

      {/* Locale */}
      <div className="space-y-3">
        <Label>Locale</Label>
        <div className="grid grid-cols-3 gap-4">
          <div className="space-y-2">
            <Label htmlFor="locale-language">Language</Label>
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
            <Label htmlFor="locale-region">Region</Label>
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
            <Label htmlFor="locale-script">Script</Label>
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
        <Label>WebGL Properties</Label>
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label htmlFor="webgl-vendor">WebGL Vendor</Label>
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
            <Label htmlFor="webgl-renderer">WebGL Renderer</Label>
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
          title="WebGL Parameters"
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
          title="WebGL2 Parameters"
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
          title="WebGL Shader Precision Formats"
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
          title="WebGL2 Shader Precision Formats"
        />
      </div>

      {/* Fonts */}
      <div className="space-y-3">
        <Label>Fonts</Label>
        <MultipleSelector
          value={
            fingerprintConfig.fonts?.map((font) => ({
              label: font,
              value: font,
            })) || []
          }
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
        <Label>Battery</Label>
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
              <Label htmlFor="battery-charging">Charging</Label>
            </div>
          </div>
          <div className="space-y-2">
            <Label htmlFor="charging-time">Charging Time</Label>
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
            <Label htmlFor="discharging-time">Discharging Time</Label>
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
    </div>
  );

  return (
    <div className={`space-y-6 ${className}`}>
      {forceAdvanced ? (
        // Advanced mode only (for editing)
        renderAdvancedForm()
      ) : (
        <Tabs value={activeTab} onValueChange={setActiveTab} className="w-full">
          <TabsList className="grid grid-cols-2 w-full">
            <TabsTrigger value="automatic">Automatic</TabsTrigger>
            <TabsTrigger value="manual">Manual</TabsTrigger>
          </TabsList>

          <TabsContent value="automatic" className="space-y-6">
            {/* Automatic Location Configuration */}
            <div className="mt-4 space-y-3">
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="auto-location"
                  checked={isAutoLocationEnabled}
                  onCheckedChange={handleAutoLocationToggle}
                />
                <Label htmlFor="auto-location">
                  Automatically configure location information based on proxy
                  configuration or your connection if no proxy provided
                </Label>
              </div>
            </div>

            {/* Screen Resolution */}
            <div className="space-y-3">
              <Label>Screen Resolution</Label>
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-2">
                  <Label htmlFor="screen-max-width">Max Width</Label>
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
                  <Label htmlFor="screen-max-height">Max Height</Label>
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
                  <Label htmlFor="screen-min-width">Min Width</Label>
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
                  <Label htmlFor="screen-min-height">Min Height</Label>
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
