"use client";

import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuTriangleAlert } from "react-icons/lu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { BrowserProfile } from "@/types";
import { RippleButton } from "./ui/ripple";

interface CamoufoxDeprecationDialogProps {
  profiles: BrowserProfile[];
}

/**
 * Warns users who still have Camoufox profiles that Camoufox support is ending.
 * Shown once per app session (this component mounts for the app lifetime), only
 * when at least one Camoufox profile exists. Not a toast — a blocking dialog so
 * the deprecation can't be missed.
 */
export function CamoufoxDeprecationDialog({
  profiles,
}: CamoufoxDeprecationDialogProps) {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const [shown, setShown] = useState(false);

  useEffect(() => {
    if (shown) return;
    const hasCamoufox = profiles.some((p) => p.browser === "camoufox");
    if (hasCamoufox) {
      setIsOpen(true);
      setShown(true);
    }
  }, [profiles, shown]);

  return (
    <Dialog open={isOpen} onOpenChange={setIsOpen}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <LuTriangleAlert className="size-5 text-warning" />
            {t("camoufoxDeprecation.title")}
          </DialogTitle>
          <DialogDescription>
            {t("camoufoxDeprecation.description")}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={() => {
              void openUrl(
                "https://github.com/zhom/donutbrowser/discussions/426",
              );
            }}
          >
            {t("common.buttons.learnMore")}
          </RippleButton>
          <RippleButton
            onClick={() => {
              setIsOpen(false);
            }}
          >
            {t("camoufoxDeprecation.acknowledge")}
          </RippleButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
