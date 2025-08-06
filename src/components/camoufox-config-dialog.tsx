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
    geoip: true,
  });
  const [isSaving, setIsSaving] = useState(false);

  // Initialize config when profile changes
  useEffect(() => {
    if (profile && profile.browser === "camoufox") {
      setConfig(
        profile.camoufox_config || {
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
        <DialogHeader className="flex-shrink-0">
          <DialogTitle>
            Configure Camoufox Settings - {profile.name}
          </DialogTitle>
        </DialogHeader>

        <ScrollArea className="flex-1 pr-6 h-[400px]">
          <div className="py-4">
            <SharedCamoufoxConfigForm
              config={config}
              onConfigChange={updateConfig}
              forceAdvanced={true}
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
