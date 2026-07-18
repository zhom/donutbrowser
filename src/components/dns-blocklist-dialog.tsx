"use client";

import { invoke } from "@tauri-apps/api/core";
import {
  open as openDialog,
  save as saveDialog,
} from "@tauri-apps/plugin-dialog";
import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuRefreshCw } from "react-icons/lu";
import { toast } from "sonner";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
import {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { translateBackendError } from "@/lib/backend-errors";
import { dnsBlocklistLabelKey } from "@/lib/dns-blocklist-levels";
import { LoadingButton } from "./loading-button";

interface BlocklistCacheStatus {
  level: string;
  display_name: string;
  entry_count: number;
  file_size_bytes: number;
  last_updated: number | null;
  is_fresh: boolean;
  is_cached: boolean;
}

interface CustomDnsConfig {
  sources: string[];
  block_domains: string[];
  allow_domains: string[];
  allowlist_mode: boolean;
  updated_at: number | null;
}

interface DnsBlocklistDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

const linesToArray = (v: string) =>
  v
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);

export function DnsBlocklistDialog({
  isOpen,
  onClose,
}: DnsBlocklistDialogProps) {
  const { t } = useTranslation();
  const [statuses, setStatuses] = useState<BlocklistCacheStatus[]>([]);
  const [isRefreshing, setIsRefreshing] = useState(false);

  const [sources, setSources] = useState("");
  const [blockDomains, setBlockDomains] = useState("");
  const [allowDomains, setAllowDomains] = useState("");
  const [allowlistMode, setAllowlistMode] = useState(false);
  const [isSaving, setIsSaving] = useState(false);

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

  const loadCustomConfig = useCallback(async () => {
    try {
      const config = await invoke<CustomDnsConfig>("get_custom_dns_config");
      setSources(config.sources.join("\n"));
      setBlockDomains(config.block_domains.join("\n"));
      setAllowDomains(config.allow_domains.join("\n"));
      setAllowlistMode(config.allowlist_mode);
    } catch (e) {
      console.error("Failed to load custom DNS config:", e);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      void loadStatuses();
      void loadCustomConfig();
    }
  }, [isOpen, loadStatuses, loadCustomConfig]);

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

  const handleSaveCustom = async () => {
    setIsSaving(true);
    try {
      const config = await invoke<CustomDnsConfig>("set_custom_dns_config", {
        sources: linesToArray(sources),
        blockDomains: linesToArray(blockDomains),
        allowDomains: linesToArray(allowDomains),
        allowlistMode,
      });
      setSources(config.sources.join("\n"));
      setBlockDomains(config.block_domains.join("\n"));
      setAllowDomains(config.allow_domains.join("\n"));
      setAllowlistMode(config.allowlist_mode);
      toast.success(t("dnsBlocklist.custom.saved"));
    } catch (e) {
      toast.error(translateBackendError(t, e));
    } finally {
      setIsSaving(false);
    }
  };

  const handleImport = async () => {
    try {
      const selected = await openDialog({
        multiple: false,
        filters: [{ name: "Rules", extensions: ["json", "txt"] }],
      });
      if (!selected || typeof selected !== "string") return;
      const content = await readTextFile(selected);
      const format = selected.toLowerCase().endsWith(".json") ? "json" : "txt";
      const config = await invoke<CustomDnsConfig>("import_custom_dns_rules", {
        content,
        format,
      });
      setSources(config.sources.join("\n"));
      setBlockDomains(config.block_domains.join("\n"));
      setAllowDomains(config.allow_domains.join("\n"));
      setAllowlistMode(config.allowlist_mode);
      toast.success(t("dnsBlocklist.custom.imported"));
    } catch (e) {
      toast.error(translateBackendError(t, e));
    }
  };

  const handleExport = async (format: "json" | "txt") => {
    try {
      const content = await invoke<string>("export_custom_dns_rules", {
        format,
      });
      const path = await saveDialog({
        defaultPath: `donut-dns-rules.${format}`,
        filters: [{ name: format.toUpperCase(), extensions: [format] }],
      });
      if (!path) return;
      await writeTextFile(path, content);
      toast.success(t("dnsBlocklist.custom.exported"));
    } catch (e) {
      toast.error(translateBackendError(t, e));
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
      <DialogContent className="flex max-h-[80vh] max-w-lg flex-col">
        <DialogHeader className="shrink-0">
          <DialogTitle>{t("dnsBlocklist.title")}</DialogTitle>
        </DialogHeader>

        <AnimatedTabs
          defaultValue="blocklists"
          className="flex min-h-0 flex-1 flex-col gap-4"
        >
          <AnimatedTabsList className="shrink-0">
            <AnimatedTabsTrigger value="blocklists">
              {t("dnsBlocklist.tabBlocklists")}
            </AnimatedTabsTrigger>
            <AnimatedTabsTrigger value="custom">
              {t("dnsBlocklist.tabCustom")}
            </AnimatedTabsTrigger>
          </AnimatedTabsList>

          <AnimatedTabsContent
            value="blocklists"
            className="min-h-0 flex-1 space-y-3 overflow-y-auto"
          >
            <p className="text-sm text-muted-foreground">
              {t("dnsBlocklist.settingsDescription")}
            </p>
            {statuses.map((status) => (
              <div
                key={status.level}
                className="flex items-center justify-between rounded-md border border-border p-3"
              >
                <div className="space-y-1">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium">
                      {t(dnsBlocklistLabelKey(status.level))}
                    </span>
                    {status.is_cached ? (
                      status.is_fresh ? (
                        <Badge variant="default" className="px-1.5 text-[10px]">
                          {t("dnsBlocklist.fresh")}
                        </Badge>
                      ) : (
                        <Badge
                          variant="secondary"
                          className="px-1.5 text-[10px]"
                        >
                          {t("dnsBlocklist.stale")}
                        </Badge>
                      )
                    ) : (
                      <Badge
                        variant="outline"
                        className="px-1.5 text-[10px] text-muted-foreground"
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
            <Button
              onClick={handleRefreshAll}
              disabled={isRefreshing}
              variant="outline"
              className="w-full"
            >
              <LuRefreshCw
                className={`mr-2 size-4 ${isRefreshing ? "animate-spin" : ""}`}
              />
              {t("dnsBlocklist.refreshAll")}
            </Button>
          </AnimatedTabsContent>

          <AnimatedTabsContent
            value="custom"
            className="min-h-0 flex-1 space-y-4 overflow-y-auto"
          >
            <p className="text-sm text-muted-foreground">
              {t("dnsBlocklist.custom.description")}
            </p>

            <div className="flex items-center justify-between gap-3 rounded-md border border-border p-3">
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium">
                  {t("dnsBlocklist.custom.allowlistModeLabel")}
                </p>
                <p className="text-[11px] text-muted-foreground">
                  {allowlistMode
                    ? t("dnsBlocklist.custom.allowlistModeOn")
                    : t("dnsBlocklist.custom.allowlistModeOff")}
                </p>
              </div>
              <AnimatedSwitch
                checked={allowlistMode}
                onCheckedChange={(v) => setAllowlistMode(v === true)}
                aria-label={t("dnsBlocklist.custom.allowlistModeLabel")}
              />
            </div>

            {!allowlistMode && (
              <div>
                <Label className="mb-1.5">
                  {t("dnsBlocklist.custom.sourcesLabel")}
                </Label>
                <Textarea
                  value={sources}
                  onChange={(e) => setSources(e.target.value)}
                  placeholder={t("dnsBlocklist.custom.sourcesPlaceholder")}
                  rows={3}
                  className="font-mono text-xs"
                />
              </div>
            )}

            {!allowlistMode && (
              <div>
                <Label className="mb-1.5">
                  {t("dnsBlocklist.custom.blockLabel")}
                </Label>
                <Textarea
                  value={blockDomains}
                  onChange={(e) => setBlockDomains(e.target.value)}
                  placeholder={t("dnsBlocklist.custom.blockPlaceholder")}
                  rows={4}
                  className="font-mono text-xs"
                />
              </div>
            )}

            <div>
              <Label className="mb-1.5">
                {allowlistMode
                  ? t("dnsBlocklist.custom.allowedOnlyLabel")
                  : t("dnsBlocklist.custom.allowLabel")}
              </Label>
              <Textarea
                value={allowDomains}
                onChange={(e) => setAllowDomains(e.target.value)}
                placeholder={t("dnsBlocklist.custom.allowPlaceholder")}
                rows={allowlistMode ? 6 : 3}
                className="font-mono text-xs"
              />
              <p className="mt-1 text-[11px] text-muted-foreground">
                {allowlistMode
                  ? t("dnsBlocklist.custom.allowedOnlyHint")
                  : t("dnsBlocklist.custom.allowHint")}
              </p>
            </div>

            <div className="flex flex-wrap items-center gap-2">
              <LoadingButton isLoading={isSaving} onClick={handleSaveCustom}>
                {t("common.buttons.save")}
              </LoadingButton>
              <Button variant="outline" onClick={handleImport}>
                {t("common.buttons.import")}
              </Button>
              <Button
                variant="outline"
                onClick={() => void handleExport("txt")}
              >
                {t("dnsBlocklist.custom.exportTxt")}
              </Button>
              <Button
                variant="outline"
                onClick={() => void handleExport("json")}
              >
                {t("dnsBlocklist.custom.exportJson")}
              </Button>
            </div>
          </AnimatedTabsContent>
        </AnimatedTabs>
      </DialogContent>
    </Dialog>
  );
}
