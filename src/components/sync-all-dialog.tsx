"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { FiWifi } from "react-icons/fi";
import { LuLayers, LuPuzzle, LuShield, LuUsers } from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import { Badge } from "@/components/ui/badge";
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
  extensions: number;
  extension_groups: number;
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

  const items = useMemo(() => {
    if (!counts) return [];
    return [
      {
        key: "proxies",
        count: counts.proxies,
        label: t("syncAll.labels.proxies"),
        Icon: FiWifi,
      },
      {
        key: "vpns",
        count: counts.vpns,
        label: t("syncAll.labels.vpns"),
        Icon: LuShield,
      },
      {
        key: "groups",
        count: counts.groups,
        label: t("syncAll.labels.groups"),
        Icon: LuUsers,
      },
      {
        key: "extensions",
        count: counts.extensions,
        label: t("syncAll.labels.extensions"),
        Icon: LuPuzzle,
      },
      {
        key: "extensionGroups",
        count: counts.extension_groups,
        label: t("syncAll.labels.extensionGroups"),
        Icon: LuLayers,
      },
    ].filter((item) => item.count > 0);
  }, [counts, t]);

  const totalCount = items.reduce((sum, item) => sum + item.count, 0);

  // Don't render anything when there's nothing to sync — the parent
  // mounts this dialog eagerly after login, so silent-close is correct.
  if (!isLoading && totalCount === 0) {
    return null;
  }

  return (
    <Dialog
      open={isOpen && (isLoading || totalCount > 0)}
      onOpenChange={onClose}
    >
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("syncAll.title")}</DialogTitle>
          <DialogDescription>{t("syncAll.description")}</DialogDescription>
        </DialogHeader>

        {isLoading ? (
          <div className="flex justify-center py-8">
            <div className="size-6 rounded-full border-2 border-current animate-spin border-t-transparent" />
          </div>
        ) : (
          <div className="grid grid-cols-2 gap-2 py-2">
            {items.map(({ key, count, label, Icon }) => (
              <div
                key={key}
                className="flex items-center gap-3 rounded-lg border border-border/60 bg-card/50 p-3 transition-colors hover:bg-card"
              >
                <div className="flex size-9 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
                  <Icon className="size-4" />
                </div>
                <div className="min-w-0 flex-1 text-sm font-medium truncate">
                  {label}
                </div>
                <Badge
                  variant="secondary"
                  className="shrink-0 tabular-nums px-2"
                >
                  {count}
                </Badge>
              </div>
            ))}
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
