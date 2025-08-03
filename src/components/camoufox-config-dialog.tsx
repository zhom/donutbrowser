"use client";

import { useEffect, useState } from "react";
import { SharedCamoufoxConfigForm } from "@/components/shared-camoufox-config-form";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { getCurrentOS } from "@/lib/browser-utils";
import type { BrowserProfile, CamoufoxConfig } from "@/types";

interface CamoufoxConfigDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
  onSave: (profile: BrowserProfile, config: CamoufoxConfig) => Promise<void>;
}

export function CamoufoxConfigDialog({
  isOpen,
  onClose,
  profile,
  onSave,
}: CamoufoxConfigDialogProps) {
  const [config, setConfig] = useState<CamoufoxConfig>({
    enable_cache: true,
    os: [getCurrentOS()],
    geoip: true,
  });
  const [isSaving, setIsSaving] = useState(false);

  // Initialize config when profile changes
  useEffect(() => {
    if (profile && profile.browser === "camoufox") {
      setConfig(
        profile.camoufox_config || {
          enable_cache: true,
          os: [getCurrentOS()],
          geoip: true,
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
          geoip: true,
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
          <div className="py-4">
            {/* OS Warning */}
            {showOSWarning && (
              <div className="mb-6 p-3 bg-amber-50 rounded-md border border-amber-200">
                <p className="text-sm text-amber-800">
                  ⚠️ Warning: Spoofing OS features is detectable by advanced
                  anti-bot systems. Some platform-specific APIs and behaviors
                  cannot be fully replicated.
                </p>
              </div>
            )}

            <SharedCamoufoxConfigForm
              config={config}
              onConfigChange={updateConfig}
            />
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
