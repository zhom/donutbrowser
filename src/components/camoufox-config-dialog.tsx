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
import type { BrowserProfile, CamoufoxConfig, CamoufoxOS } from "@/types";

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
  isRunning?: boolean;
}

export function CamoufoxConfigDialog({
  isOpen,
  onClose,
  profile,
  onSave,
  isRunning = false,
}: CamoufoxConfigDialogProps) {
  const [config, setConfig] = useState<CamoufoxConfig>(() => ({
    geoip: true,
    os: getCurrentOS(),
  }));
  const [isSaving, setIsSaving] = useState(false);

  // Initialize config when profile changes
  useEffect(() => {
    if (profile && profile.browser === "camoufox") {
      setConfig(
        profile.camoufox_config || {
          geoip: true,
          os: getCurrentOS(),
        },
      );
    }
  }, [profile]);

  const updateConfig = (key: keyof CamoufoxConfig, value: unknown) => {
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
      await onSave(profile, config);
      onClose();
    } catch (error) {
      console.error("Failed to save camoufox config:", error);
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
    if (profile && profile.browser === "camoufox") {
      setConfig(
        profile.camoufox_config || {
          geoip: true,
          os: getCurrentOS(),
        },
      );
    }
    onClose();
  };

  if (!profile || profile.browser !== "camoufox") {
    return null;
  }

  // No OS warning needed anymore since we removed OS selection

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-3xl max-h-[90vh] flex flex-col">
        <DialogHeader className="shrink-0">
          <DialogTitle>
            {isRunning ? "View" : "Configure"} Fingerprint Settings -{" "}
            {profile.name}
          </DialogTitle>
        </DialogHeader>

        <ScrollArea className="flex-1 h-[300px]">
          <div className="py-4">
            <SharedCamoufoxConfigForm
              config={config}
              onConfigChange={updateConfig}
              forceAdvanced={true}
              readOnly={isRunning}
            />
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
