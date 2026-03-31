"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuRefreshCw } from "react-icons/lu";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

interface BlocklistCacheStatus {
  level: string;
  display_name: string;
  entry_count: number;
  file_size_bytes: number;
  last_updated: number | null;
  is_fresh: boolean;
  is_cached: boolean;
}

interface DnsBlocklistDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function DnsBlocklistDialog({
  isOpen,
  onClose,
}: DnsBlocklistDialogProps) {
  const { t } = useTranslation();
  const [statuses, setStatuses] = useState<BlocklistCacheStatus[]>([]);
  const [isRefreshing, setIsRefreshing] = useState(false);

  const loadStatuses = useCallback(async () => {
    try {
      const result = await invoke<BlocklistCacheStatus[]>(
        "get_dns_blocklist_cache_status",
      );
      setStatuses(result);
    } catch (e) {
      console.error("Failed to load blocklist status:", e);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      void loadStatuses();
    }
  }, [isOpen, loadStatuses]);

  const handleRefreshAll = async () => {
    setIsRefreshing(true);
    try {
      await invoke("refresh_dns_blocklists");
      await loadStatuses();
    } catch (e) {
      console.error("Failed to refresh blocklists:", e);
    } finally {
      setIsRefreshing(false);
    }
  };

  const formatSize = (bytes: number) => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  };

  const formatDate = (timestamp: number | null) => {
    if (!timestamp) return t("dnsBlocklist.notCached");
    return new Date(timestamp * 1000).toLocaleString();
  };

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("dnsBlocklist.title")}</DialogTitle>
        </DialogHeader>

        <p className="text-sm text-muted-foreground">
          {t("dnsBlocklist.settingsDescription")}
        </p>

        <div className="space-y-3">
          {statuses.map((status) => (
            <div
              key={status.level}
              className="flex items-center justify-between rounded-md border border-border p-3"
            >
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium">
                    {status.display_name}
                  </span>
                  {status.is_cached ? (
                    status.is_fresh ? (
                      <Badge variant="default" className="text-[10px] px-1.5">
                        {t("dnsBlocklist.fresh")}
                      </Badge>
                    ) : (
                      <Badge variant="secondary" className="text-[10px] px-1.5">
                        {t("dnsBlocklist.stale")}
                      </Badge>
                    )
                  ) : (
                    <Badge
                      variant="outline"
                      className="text-[10px] px-1.5 text-muted-foreground"
                    >
                      {t("dnsBlocklist.notCached")}
                    </Badge>
                  )}
                </div>
                {status.is_cached && (
                  <div className="text-xs text-muted-foreground">
                    {status.entry_count.toLocaleString()}{" "}
                    {t("dnsBlocklist.domains")} &middot;{" "}
                    {formatSize(status.file_size_bytes)} &middot;{" "}
                    {formatDate(status.last_updated)}
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>

        <Button
          onClick={handleRefreshAll}
          disabled={isRefreshing}
          variant="outline"
          className="w-full"
        >
          <LuRefreshCw
            className={`mr-2 h-4 w-4 ${isRefreshing ? "animate-spin" : ""}`}
          />
          {t("dnsBlocklist.refreshAll")}
        </Button>
      </DialogContent>
    </Dialog>
  );
}
