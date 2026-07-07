"use client";

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { SharedFingerprintConfigForm } from "@/components/shared-fingerprint-config-form";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { BrowserProfile, WayfernConfig, WayfernOS } from "@/types";
import { LoadingButton } from "./loading-button";
import { RippleButton } from "./ui/ripple";

const getCurrentOS = (): WayfernOS => {
  if (typeof navigator === "undefined") return "linux";
  const platform = navigator.platform.toLowerCase();
  if (platform.includes("win")) return "windows";
  if (platform.includes("mac")) return "macos";
  return "linux";
};

interface WayfernConfigDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
  onSave: (profile: BrowserProfile, config: WayfernConfig) => Promise<void>;
  isRunning?: boolean;
  crossOsUnlocked?: boolean;
}

export function WayfernConfigDialog({
  isOpen,
  onClose,
  profile,
  onSave,
  isRunning = false,
  crossOsUnlocked = false,
}: WayfernConfigDialogProps) {
  const { t } = useTranslation();
  const [config, setConfig] = useState<WayfernConfig>(() => ({
    geoip: true,
    os: getCurrentOS(),
  }));
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    if (profile?.browser === "wayfern") {
      setConfig(
        profile.wayfern_config || {
          geoip: true,
          os: getCurrentOS(),
        },
      );
    }
  }, [profile]);

  const updateConfig = (key: keyof WayfernConfig, value: unknown) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const handleSave = async () => {
    if (!profile) return;

    if (config.fingerprint) {
      try {
        JSON.parse(config.fingerprint);
      } catch (_error) {
        const { toast } = await import("sonner");
        toast.error(t("wayfernConfigDialog.invalidFingerprint"), {
          description: t("wayfernConfigDialog.invalidFingerprintDescription"),
        });
        return;
      }
    }

    setIsSaving(true);
    try {
      await onSave(profile, config);
      onClose();
    } catch (error) {
      console.error("Failed to save config:", error);
      const { toast } = await import("sonner");
      toast.error(t("wayfernConfigDialog.saveFailed"), {
        description:
          error instanceof Error
            ? error.message
            : t("wayfernConfigDialog.unknownError"),
      });
    } finally {
      setIsSaving(false);
    }
  };

  const handleClose = () => {
    if (profile?.browser === "wayfern") {
      setConfig(
        profile.wayfern_config || {
          geoip: true,
          os: getCurrentOS(),
        },
      );
    }
    onClose();
  };

  if (profile?.browser !== "wayfern") {
    return null;
  }

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="flex h-[min(85vh,52rem)] max-w-3xl flex-col">
        <DialogHeader className="shrink-0">
          <DialogTitle>
            {isRunning
              ? t("wayfernConfigDialog.titleView", {
                  name: profile.name,
                  browser: "Wayfern",
                })
              : t("wayfernConfigDialog.titleConfigure", {
                  name: profile.name,
                  browser: "Wayfern",
                })}
          </DialogTitle>
        </DialogHeader>

        <ScrollArea className="min-h-0 flex-1">
          <div className="py-4">
            <SharedFingerprintConfigForm
              config={config}
              onConfigChange={updateConfig}
              forceAdvanced={true}
              readOnly={isRunning}
              crossOsUnlocked={crossOsUnlocked}
              limitedMode={!crossOsUnlocked}
              profileVersion={profile.version}
              profileBrowser="wayfern"
            />
          </div>
        </ScrollArea>

        <DialogFooter className="shrink-0 border-t pt-4">
          <RippleButton variant="outline" onClick={handleClose}>
            {isRunning ? t("common.buttons.close") : t("common.buttons.cancel")}
          </RippleButton>
          {!isRunning && (
            <LoadingButton
              isLoading={isSaving}
              onClick={handleSave}
              disabled={isSaving}
            >
              {t("common.buttons.save")}
            </LoadingButton>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
