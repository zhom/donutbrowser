"use client";

import { useEffect, useState } from "react";
import { SharedCamoufoxConfigForm } from "@/components/shared-camoufox-config-form";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { WayfernConfigForm } from "@/components/wayfern-config-form";
import type {
  BrowserProfile,
  CamoufoxConfig,
  CamoufoxOS,
  WayfernConfig,
} from "@/types";

const getCurrentOS = (): CamoufoxOS => {
  if (typeof navigator === "undefined") return "linux";
  const platform = navigator.platform.toLowerCase();
  if (platform.includes("win")) return "windows";
  if (platform.includes("mac")) return "macos";
  return "linux";
};

import { LoadingButton } from "./loading-button";
import { RippleButton } from "./ui/ripple";

interface CamoufoxConfigDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
  onSave: (profile: BrowserProfile, config: CamoufoxConfig) => Promise<void>;
  onSaveWayfern?: (
    profile: BrowserProfile,
    config: CamoufoxConfig,
  ) => Promise<void>;
  isRunning?: boolean;
  crossOsUnlocked?: boolean;
}

export function CamoufoxConfigDialog({
  isOpen,
  onClose,
  profile,
  onSave,
  onSaveWayfern,
  isRunning = false,
  crossOsUnlocked = false,
}: CamoufoxConfigDialogProps) {
  // Use union type to support both Camoufox and Wayfern configs
  const [config, setConfig] = useState<CamoufoxConfig | WayfernConfig>(() => ({
    geoip: true,
    os: getCurrentOS(),
  }));
  const [isSaving, setIsSaving] = useState(false);

  const isAntiDetectBrowser =
    profile?.browser === "camoufox" || profile?.browser === "wayfern";

  // Initialize config when profile changes
  useEffect(() => {
    if (profile && isAntiDetectBrowser) {
      const profileConfig =
        profile.browser === "wayfern"
          ? profile.wayfern_config
          : profile.camoufox_config;
      setConfig(
        profileConfig || {
          geoip: true,
          os: getCurrentOS(),
        },
      );
    }
  }, [profile, isAntiDetectBrowser]);

  const updateConfig = (
    key: keyof CamoufoxConfig | keyof WayfernConfig,
    value: unknown,
  ) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const handleSave = async () => {
    if (!profile) return;

    // Validate fingerprint JSON if it exists
    if (config.fingerprint) {
      try {
        JSON.parse(config.fingerprint);
      } catch (_error) {
        const { toast } = await import("sonner");
        toast.error("Invalid fingerprint configuration", {
          description:
            "The fingerprint configuration contains invalid JSON. Please check your advanced settings.",
        });
        return;
      }
    }

    setIsSaving(true);
    try {
      if (profile.browser === "wayfern" && onSaveWayfern) {
        await onSaveWayfern(profile, config as CamoufoxConfig);
      } else {
        await onSave(profile, config as CamoufoxConfig);
      }
      onClose();
    } catch (error) {
      console.error("Failed to save config:", error);
      const { toast } = await import("sonner");
      toast.error("Failed to save configuration", {
        description:
          error instanceof Error ? error.message : "Unknown error occurred",
      });
    } finally {
      setIsSaving(false);
    }
  };

  const handleClose = () => {
    // Reset config to original when closing without saving
    if (profile && isAntiDetectBrowser) {
      const profileConfig =
        profile.browser === "wayfern"
          ? profile.wayfern_config
          : profile.camoufox_config;
      setConfig(
        profileConfig || {
          geoip: true,
          os: getCurrentOS(),
        },
      );
    }
    onClose();
  };

  if (!profile || !isAntiDetectBrowser) {
    return null;
  }

  const browserName = profile.browser === "wayfern" ? "Wayfern" : "Camoufox";

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-3xl max-h-[90vh] flex flex-col">
        <DialogHeader className="shrink-0">
          <DialogTitle>
            {isRunning ? "View" : "Configure"} Fingerprint Settings -{" "}
            {profile.name} ({browserName})
          </DialogTitle>
        </DialogHeader>

        <ScrollArea className="flex-1 h-[300px]">
          <div className="py-4">
            {profile.browser === "wayfern" ? (
              <WayfernConfigForm
                config={config as WayfernConfig}
                onConfigChange={updateConfig}
                forceAdvanced={true}
                readOnly={isRunning}
                crossOsUnlocked={crossOsUnlocked}
              />
            ) : (
              <SharedCamoufoxConfigForm
                config={config as CamoufoxConfig}
                onConfigChange={updateConfig}
                forceAdvanced={true}
                readOnly={isRunning}
                browserType="camoufox"
                crossOsUnlocked={crossOsUnlocked}
              />
            )}
          </div>
        </ScrollArea>

        <DialogFooter className="shrink-0 pt-4 border-t">
          <RippleButton variant="outline" onClick={handleClose}>
            {isRunning ? "Close" : "Cancel"}
          </RippleButton>
          {!isRunning && (
            <LoadingButton
              isLoading={isSaving}
              onClick={handleSave}
              disabled={isSaving}
            >
              Save
            </LoadingButton>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
