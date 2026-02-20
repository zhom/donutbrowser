"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LoadingButton } from "@/components/loading-button";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";

interface UnsyncedEntityCounts {
  proxies: number;
  groups: number;
  vpns: number;
}

interface SyncAllDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function SyncAllDialog({ isOpen, onClose }: SyncAllDialogProps) {
  const { t } = useTranslation();
  const [counts, setCounts] = useState<UnsyncedEntityCounts | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isEnabling, setIsEnabling] = useState(false);

  const loadCounts = useCallback(async () => {
    setIsLoading(true);
    try {
      const result = await invoke<UnsyncedEntityCounts>(
        "get_unsynced_entity_counts",
      );
      setCounts(result);
    } catch (error) {
      console.error("Failed to get unsynced entity counts:", error);
      setCounts(null);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      void loadCounts();
    }
  }, [isOpen, loadCounts]);

  const handleEnableAll = useCallback(async () => {
    setIsEnabling(true);
    try {
      await invoke("enable_sync_for_all_entities");
      showSuccessToast(t("syncAll.success"));
      onClose();
    } catch (error) {
      console.error("Failed to enable sync for all entities:", error);
      showErrorToast(String(error));
    } finally {
      setIsEnabling(false);
    }
  }, [onClose, t]);

  const totalCount =
    (counts?.proxies ?? 0) + (counts?.groups ?? 0) + (counts?.vpns ?? 0);

  // Don't show if there's nothing to sync
  if (!isLoading && totalCount === 0) {
    return null;
  }

  const parts: string[] = [];
  if (counts?.proxies && counts.proxies > 0) {
    parts.push(t("syncAll.proxies", { count: counts.proxies }));
  }
  if (counts?.groups && counts.groups > 0) {
    parts.push(t("syncAll.groups", { count: counts.groups }));
  }
  if (counts?.vpns && counts.vpns > 0) {
    parts.push(t("syncAll.vpns", { count: counts.vpns }));
  }

  return (
    <Dialog open={isOpen && totalCount > 0} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("syncAll.title")}</DialogTitle>
          <DialogDescription>{t("syncAll.description")}</DialogDescription>
        </DialogHeader>

        {isLoading ? (
          <div className="flex justify-center py-8">
            <div className="w-6 h-6 rounded-full border-2 border-current animate-spin border-t-transparent" />
          </div>
        ) : (
          <div className="py-4">
            <p className="text-sm text-muted-foreground">
              {t("syncAll.itemsList", { items: parts.join(", ") })}
            </p>
          </div>
        )}

        <DialogFooter className="flex gap-2">
          <Button variant="outline" onClick={onClose} disabled={isEnabling}>
            {t("syncAll.skip")}
          </Button>
          <LoadingButton
            onClick={handleEnableAll}
            isLoading={isEnabling}
            disabled={isLoading}
          >
            {t("syncAll.enableAll")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
