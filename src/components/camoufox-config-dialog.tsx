"use client";

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { BrowserProfile, CamoufoxConfig } from "@/types";

interface CamoufoxConfigDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
  onSave: (profile: BrowserProfile, config: CamoufoxConfig) => Promise<void>;
}

const osOptions = [
  { value: "windows", label: "Windows" },
  { value: "macos", label: "macOS" },
  { value: "linux", label: "Linux" },
];

const timezoneOptions = [
  { value: "America/New_York", label: "America/New_York" },
  { value: "America/Los_Angeles", label: "America/Los_Angeles" },
  { value: "Europe/London", label: "Europe/London" },
  { value: "Europe/Paris", label: "Europe/Paris" },
  { value: "Asia/Tokyo", label: "Asia/Tokyo" },
  { value: "Asia/Shanghai", label: "Asia/Shanghai" },
  { value: "Australia/Sydney", label: "Australia/Sydney" },
];

const localeOptions = [
  { value: "en-US", label: "English (US)" },
  { value: "en-GB", label: "English (UK)" },
  { value: "fr-FR", label: "French" },
  { value: "de-DE", label: "German" },
  { value: "es-ES", label: "Spanish" },
  { value: "it-IT", label: "Italian" },
  { value: "ja-JP", label: "Japanese" },
  { value: "zh-CN", label: "Chinese (Simplified)" },
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

export function CamoufoxConfigDialog({
  isOpen,
  onClose,
  profile,
  onSave,
}: CamoufoxConfigDialogProps) {
  const [config, setConfig] = useState<CamoufoxConfig>({
    enable_cache: true,
    os: [getCurrentOS()],
  });
  const [isSaving, setIsSaving] = useState(false);

  // Initialize config when profile changes
  useEffect(() => {
    if (profile && profile.browser === "camoufox") {
      setConfig(
        profile.camoufox_config || {
          enable_cache: true,
          os: [getCurrentOS()],
        },
      );
    }
  }, [profile]);

  const updateConfig = (key: keyof CamoufoxConfig, value: unknown) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const handleSave = async () => {
    if (!profile) return;

    setIsSaving(true);
    try {
      await onSave(profile, config);
      onClose();
    } catch (error) {
      console.error("Failed to save camoufox config:", error);
    } finally {
      setIsSaving(false);
    }
  };

  const handleClose = () => {
    // Reset config to original when closing without saving
    if (profile && profile.browser === "camoufox") {
      setConfig(
        profile.camoufox_config || {
          enable_cache: true,
          os: [getCurrentOS()],
        },
      );
    }
    onClose();
  };

  if (!profile || profile.browser !== "camoufox") {
    return null;
  }

  // Get the selected OS for warning
  const selectedOS = config.os?.[0];
  const currentOS = getCurrentOS();
  const showOSWarning =
    selectedOS && selectedOS !== currentOS && currentOS !== "unknown";

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-3xl max-h-[90vh] flex flex-col">
        <DialogHeader className="flex-shrink-0">
          <DialogTitle>
            Configure Camoufox Settings - {profile.name}
          </DialogTitle>
        </DialogHeader>

        <ScrollArea className="flex-1 pr-6 h-[320px]">
          <div className="py-4 space-y-6">
            {/* Operating System */}
            <div className="space-y-3">
              <Label>Operating System Fingerprint</Label>
              <Select
                value={selectedOS || ""}
                onValueChange={(value: string) => updateConfig("os", [value])}
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
                <div className="p-3 bg-amber-50 rounded-md border border-amber-200">
                  <p className="text-sm text-amber-800">
                    ⚠️ Warning: Spoofing OS features is detectable by advanced
                    anti-bot systems. Some platform-specific APIs and behaviors
                    cannot be fully replicated.
                  </p>
                </div>
              )}
            </div>

            {/* Blocking Options */}
            <div className="space-y-3">
              <Label>Privacy & Blocking</Label>
              <div className="space-y-2">
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="block-images"
                    checked={config.block_images || false}
                    onCheckedChange={(checked) =>
                      updateConfig("block_images", checked)
                    }
                  />
                  <Label htmlFor="block-images">Block Images</Label>
                </div>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="block-webrtc"
                    checked={config.block_webrtc || false}
                    onCheckedChange={(checked) =>
                      updateConfig("block_webrtc", checked)
                    }
                  />
                  <Label htmlFor="block-webrtc">Block WebRTC</Label>
                </div>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="block-webgl"
                    checked={config.block_webgl || false}
                    onCheckedChange={(checked) =>
                      updateConfig("block_webgl", checked)
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
                      updateConfig("country", e.target.value || undefined)
                    }
                    placeholder="e.g., US, GB, DE"
                  />
                </div>
                <div className="space-y-2">
                  <Label>Timezone</Label>
                  <Select
                    value={config.timezone || "auto"}
                    onValueChange={(value) =>
                      updateConfig(
                        "timezone",
                        value === "auto" ? undefined : value,
                      )
                    }
                  >
                    <SelectTrigger>
                      <SelectValue placeholder="Select timezone" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="auto">Auto</SelectItem>
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
                      updateConfig(
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
                      updateConfig(
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
                  updateConfig("locale", value ? [value] : undefined)
                }
              >
                <SelectTrigger>
                  <SelectValue placeholder="Select locale" />
                </SelectTrigger>
                <SelectContent>
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
                      updateConfig(
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
                      updateConfig(
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
                      updateConfig(
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
                      updateConfig(
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
                      updateConfig(
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
                      updateConfig(
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
                      updateConfig("enable_cache", checked)
                    }
                  />
                  <Label htmlFor="enable-cache">Enable Browser Cache</Label>
                </div>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="main-world-eval"
                    checked={config.main_world_eval || false}
                    onCheckedChange={(checked) =>
                      updateConfig("main_world_eval", checked)
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
                      updateConfig("webgl_vendor", e.target.value || undefined)
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
                      updateConfig(
                        "webgl_renderer",
                        e.target.value || undefined,
                      )
                    }
                    placeholder="e.g., Intel Iris OpenGL Engine"
                  />
                </div>
              </div>
            </div>

            {/* Debug Options */}
            <div className="space-y-3">
              <Label>Debug Options</Label>
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="debug"
                  checked={config.debug || false}
                  onCheckedChange={(checked) => updateConfig("debug", checked)}
                />
                <Label htmlFor="debug">Enable Debug Mode</Label>
              </div>
            </div>
          </div>
        </ScrollArea>

        <DialogFooter className="flex-shrink-0 pt-4 border-t">
          <Button variant="outline" onClick={handleClose}>
            Cancel
          </Button>
          <Button onClick={handleSave} disabled={isSaving}>
            {isSaving ? "Saving..." : "Save Configuration"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
