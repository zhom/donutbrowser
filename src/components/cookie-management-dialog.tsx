"use client";

import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "@tauri-apps/plugin-fs";
import { useCallback, useEffect, useMemo, useState } from "react";
import { LuChevronDown, LuChevronRight, LuUpload } from "react-icons/lu";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { RippleButton } from "@/components/ui/ripple";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type {
  BrowserProfile,
  CookieReadResult,
  DomainCookies,
  UnifiedCookie,
} from "@/types";

interface CookieImportResult {
  cookies_imported: number;
  cookies_replaced: number;
  errors: string[];
}

interface CookieManagementDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
  initialTab?: "import" | "export";
}

type SelectionState = {
  [domain: string]: {
    allSelected: boolean;
    cookies: Set<string>;
  };
};

const countCookies = (content: string): number => {
  const trimmed = content.trim();
  if (trimmed.startsWith("[")) {
    try {
      const arr = JSON.parse(trimmed);
      if (Array.isArray(arr)) return arr.length;
    } catch {
      // Fall through to Netscape counting
    }
  }
  return content.split("\n").filter((line) => {
    const l = line.trim();
    return l && !l.startsWith("#");
  }).length;
};

function formatJsonCookies(cookies: UnifiedCookie[]): string {
  const arr = cookies.map((c) => {
    const sameSite =
      c.same_site === 1
        ? "lax"
        : c.same_site === 2
          ? "strict"
          : "no_restriction";
    return {
      name: c.name,
      value: c.value,
      domain: c.domain,
      path: c.path,
      secure: c.is_secure,
      httpOnly: c.is_http_only,
      sameSite,
      expirationDate: c.expires,
      session: c.expires === 0,
      hostOnly: !c.domain.startsWith("."),
    };
  });
  return JSON.stringify(arr, null, 2);
}

function formatNetscapeCookies(cookies: UnifiedCookie[]): string {
  const lines = ["# Netscape HTTP Cookie File"];
  for (const c of cookies) {
    const flag = c.domain.startsWith(".") ? "TRUE" : "FALSE";
    const secure = c.is_secure ? "TRUE" : "FALSE";
    lines.push(
      `${c.domain}\t${flag}\t${c.path}\t${secure}\t${c.expires}\t${c.name}\t${c.value}`,
    );
  }
  return lines.join("\n");
}

function initSelectionFromCookieData(data: CookieReadResult): SelectionState {
  const sel: SelectionState = {};
  for (const d of data.domains) {
    sel[d.domain] = {
      allSelected: true,
      cookies: new Set(d.cookies.map((c) => c.name)),
    };
  }
  return sel;
}

export function CookieManagementDialog({
  isOpen,
  onClose,
  profile,
  initialTab = "import",
}: CookieManagementDialogProps) {
  // Import state
  const [fileContent, setFileContent] = useState<string | null>(null);
  const [fileName, setFileName] = useState<string | null>(null);
  const [cookieCount, setCookieCount] = useState(0);
  const [isImporting, setIsImporting] = useState(false);
  const [importResult, setImportResult] = useState<CookieImportResult | null>(
    null,
  );

  // Export state
  const [format, setFormat] = useState<"netscape" | "json">("json");
  const [isExporting, setIsExporting] = useState(false);
  const [exportCookieData, setExportCookieData] =
    useState<CookieReadResult | null>(null);
  const [isLoadingExportCookies, setIsLoadingExportCookies] = useState(false);
  const [exportSelection, setExportSelection] = useState<SelectionState>({});
  const [expandedDomains, setExpandedDomains] = useState<Set<string>>(
    new Set(),
  );
  const [activeTab, setActiveTab] = useState<string>(initialTab);

  const selectedExportCount = useMemo(() => {
    let count = 0;
    for (const domain of Object.keys(exportSelection)) {
      const ds = exportSelection[domain];
      if (ds.allSelected) {
        const domainData = exportCookieData?.domains.find(
          (d) => d.domain === domain,
        );
        count += domainData?.cookie_count || 0;
      } else {
        count += ds.cookies.size;
      }
    }
    return count;
  }, [exportSelection, exportCookieData]);

  const loadExportCookies = useCallback(
    async (profileId: string) => {
      if (exportCookieData) return;
      setIsLoadingExportCookies(true);
      try {
        const result = await invoke<CookieReadResult>("read_profile_cookies", {
          profileId,
        });
        setExportCookieData(result);
        setExportSelection(initSelectionFromCookieData(result));
      } catch (err) {
        toast.error(
          `Failed to load cookies: ${err instanceof Error ? err.message : String(err)}`,
        );
      } finally {
        setIsLoadingExportCookies(false);
      }
    },
    [exportCookieData],
  );

  useEffect(() => {
    if (activeTab === "export" && profile && !exportCookieData) {
      void loadExportCookies(profile.id);
    }
  }, [activeTab, profile, exportCookieData, loadExportCookies]);

  const resetImportState = useCallback(() => {
    setFileContent(null);
    setFileName(null);
    setCookieCount(0);
    setIsImporting(false);
    setImportResult(null);
  }, []);

  const resetExportState = useCallback(() => {
    setFormat("json");
    setIsExporting(false);
    setExportCookieData(null);
    setExportSelection({});
    setExpandedDomains(new Set());
  }, []);

  const handleClose = useCallback(() => {
    resetImportState();
    resetExportState();
    setActiveTab(initialTab);
    onClose();
  }, [resetImportState, resetExportState, onClose, initialTab]);

  const handleTabChange = useCallback(
    (tab: string) => {
      setActiveTab(tab);
      resetImportState();
      if (tab !== "export") {
        resetExportState();
      }
    },
    [resetImportState, resetExportState],
  );

  const handleFileRead = useCallback((file: File) => {
    const reader = new FileReader();
    reader.onload = (e) => {
      const content = e.target?.result as string;
      setFileContent(content);
      setFileName(file.name);
      setCookieCount(countCookies(content));
    };
    reader.onerror = () => {
      toast.error("Failed to read file");
    };
    reader.readAsText(file);
  }, []);

  const handleImport = useCallback(async () => {
    if (!fileContent || !profile) return;
    setIsImporting(true);
    try {
      const result = await invoke<CookieImportResult>(
        "import_cookies_from_file",
        {
          profileId: profile.id,
          content: fileContent,
        },
      );
      setImportResult(result);
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
    } finally {
      setIsImporting(false);
    }
  }, [fileContent, profile]);

  const getSelectedCookies = useCallback((): UnifiedCookie[] => {
    if (!exportCookieData) return [];
    const result: UnifiedCookie[] = [];
    for (const domain of exportCookieData.domains) {
      const ds = exportSelection[domain.domain];
      if (!ds) continue;
      if (ds.allSelected) {
        result.push(...domain.cookies);
      } else {
        result.push(...domain.cookies.filter((c) => ds.cookies.has(c.name)));
      }
    }
    return result;
  }, [exportCookieData, exportSelection]);

  const handleExport = useCallback(async () => {
    if (!profile) return;
    setIsExporting(true);
    try {
      const cookies = getSelectedCookies();
      const content =
        format === "json"
          ? formatJsonCookies(cookies)
          : formatNetscapeCookies(cookies);

      const ext = format === "json" ? "json" : "txt";
      const defaultName = `${profile.name}_cookies.${ext}`;

      const filePath = await save({
        defaultPath: defaultName,
        filters: [
          {
            name: format === "json" ? "JSON" : "Text",
            extensions: [ext],
          },
        ],
      });

      if (!filePath) {
        setIsExporting(false);
        return;
      }

      await writeTextFile(filePath, content);
      toast.success("Cookies exported successfully");
      handleClose();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
    } finally {
      setIsExporting(false);
    }
  }, [profile, format, getSelectedCookies, handleClose]);

  const toggleDomain = useCallback(
    (domain: string, cookies: UnifiedCookie[]) => {
      setExportSelection((prev) => {
        const current = prev[domain];
        if (current?.allSelected) {
          const next = { ...prev };
          delete next[domain];
          return next;
        }
        return {
          ...prev,
          [domain]: {
            allSelected: true,
            cookies: new Set(cookies.map((c) => c.name)),
          },
        };
      });
    },
    [],
  );

  const toggleCookie = useCallback(
    (domain: string, cookieName: string, totalCookies: number) => {
      setExportSelection((prev) => {
        const current = prev[domain] || {
          allSelected: false,
          cookies: new Set<string>(),
        };
        const newCookies = new Set(current.cookies);
        if (newCookies.has(cookieName)) {
          newCookies.delete(cookieName);
        } else {
          newCookies.add(cookieName);
        }
        if (newCookies.size === 0) {
          const next = { ...prev };
          delete next[domain];
          return next;
        }
        return {
          ...prev,
          [domain]: {
            allSelected: newCookies.size === totalCookies,
            cookies: newCookies,
          },
        };
      });
    },
    [],
  );

  const toggleExpand = useCallback((domain: string) => {
    setExpandedDomains((prev) => {
      const next = new Set(prev);
      if (next.has(domain)) {
        next.delete(domain);
      } else {
        next.add(domain);
      }
      return next;
    });
  }, []);

  const toggleSelectAll = useCallback(() => {
    if (!exportCookieData) return;
    if (selectedExportCount === exportCookieData.total_count) {
      setExportSelection({});
    } else {
      setExportSelection(initSelectionFromCookieData(exportCookieData));
    }
  }, [exportCookieData, selectedExportCount]);

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Cookie Management</DialogTitle>
        </DialogHeader>

        <Tabs
          defaultValue={initialTab}
          onValueChange={handleTabChange}
          className="w-full"
        >
          <TabsList className="grid w-full grid-cols-2">
            <TabsTrigger value="import">Import</TabsTrigger>
            <TabsTrigger value="export">Export</TabsTrigger>
          </TabsList>

          <TabsContent value="import" className="space-y-4 mt-4">
            {!fileContent && (
              <div className="space-y-4">
                <p className="text-sm text-muted-foreground">
                  Import cookies from a Netscape or JSON format file.
                </p>
                <div
                  role="button"
                  tabIndex={0}
                  className="flex flex-col items-center justify-center border-2 border-dashed rounded-lg p-8 transition-colors cursor-pointer border-muted-foreground/25 hover:border-muted-foreground/50"
                  onClick={() =>
                    document.getElementById("cookie-file-input")?.click()
                  }
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      document.getElementById("cookie-file-input")?.click();
                    }
                  }}
                >
                  <LuUpload className="w-10 h-10 text-muted-foreground mb-4" />
                  <p className="text-sm text-muted-foreground text-center">
                    Click to choose a cookie file
                    <br />
                    <span className="text-xs">(.txt, .cookies, or .json)</span>
                  </p>
                  <input
                    id="cookie-file-input"
                    type="file"
                    accept=".txt,.cookies,.json"
                    className="hidden"
                    onChange={(e) => {
                      const file = e.target.files?.[0];
                      if (file) handleFileRead(file);
                      e.target.value = "";
                    }}
                  />
                </div>
              </div>
            )}

            {fileContent && !importResult && (
              <div className="space-y-4">
                <div className="flex items-center gap-3 p-4 bg-muted/30 rounded-lg">
                  <div>
                    <div className="font-medium">{fileName}</div>
                    <div className="text-sm text-muted-foreground">
                      {cookieCount} cookies found
                    </div>
                  </div>
                </div>
                <div className="flex justify-end gap-2">
                  <RippleButton variant="outline" onClick={resetImportState}>
                    Back
                  </RippleButton>
                  <LoadingButton
                    isLoading={isImporting}
                    onClick={() => void handleImport()}
                    disabled={cookieCount === 0}
                  >
                    Import
                  </LoadingButton>
                </div>
              </div>
            )}

            {importResult && (
              <div className="space-y-4">
                <div className="p-4 rounded-lg bg-green-500/10">
                  <div className="font-medium text-green-600 dark:text-green-400">
                    Successfully imported {importResult.cookies_imported}{" "}
                    cookies ({importResult.cookies_replaced} replaced)
                  </div>
                  {importResult.errors.length > 0 && (
                    <div className="mt-2 text-sm text-muted-foreground">
                      {importResult.errors.length} line(s) skipped
                    </div>
                  )}
                </div>
                <div className="flex justify-end">
                  <RippleButton onClick={handleClose}>Done</RippleButton>
                </div>
              </div>
            )}
          </TabsContent>

          <TabsContent value="export" className="space-y-3 mt-4">
            <div className="space-y-2">
              <Label>Format</Label>
              <Select
                value={format}
                onValueChange={(v) => setFormat(v as "netscape" | "json")}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="json">JSON</SelectItem>
                  <SelectItem value="netscape">Netscape TXT</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <Label>
                  Cookies{" "}
                  {exportCookieData && (
                    <span className="text-muted-foreground font-normal">
                      ({selectedExportCount} of {exportCookieData.total_count}{" "}
                      selected)
                    </span>
                  )}
                </Label>
                {exportCookieData && exportCookieData.total_count > 0 && (
                  <button
                    type="button"
                    className="text-xs text-muted-foreground hover:text-foreground transition-colors"
                    onClick={toggleSelectAll}
                  >
                    {selectedExportCount === exportCookieData.total_count
                      ? "Deselect all"
                      : "Select all"}
                  </button>
                )}
              </div>

              {isLoadingExportCookies ? (
                <div className="flex items-center justify-center h-24">
                  <div className="animate-spin h-5 w-5 border-2 border-primary border-t-transparent rounded-full" />
                </div>
              ) : !exportCookieData || exportCookieData.domains.length === 0 ? (
                <div className="p-4 text-center text-sm text-muted-foreground border rounded-md">
                  No cookies found in this profile
                </div>
              ) : (
                <ScrollArea className="h-[200px] border rounded-md">
                  <div className="p-2 space-y-1">
                    {exportCookieData.domains.map((domain) => (
                      <ExportDomainRow
                        key={domain.domain}
                        domain={domain}
                        selection={exportSelection}
                        isExpanded={expandedDomains.has(domain.domain)}
                        onToggleDomain={toggleDomain}
                        onToggleCookie={toggleCookie}
                        onToggleExpand={toggleExpand}
                      />
                    ))}
                  </div>
                </ScrollArea>
              )}
            </div>

            <div className="flex justify-end gap-2">
              <RippleButton variant="outline" onClick={handleClose}>
                Cancel
              </RippleButton>
              <LoadingButton
                isLoading={isExporting}
                onClick={() => void handleExport()}
                disabled={selectedExportCount === 0}
              >
                Export
              </LoadingButton>
            </div>
          </TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}

interface ExportDomainRowProps {
  domain: DomainCookies;
  selection: SelectionState;
  isExpanded: boolean;
  onToggleDomain: (domain: string, cookies: UnifiedCookie[]) => void;
  onToggleCookie: (
    domain: string,
    cookieName: string,
    totalCookies: number,
  ) => void;
  onToggleExpand: (domain: string) => void;
}

function ExportDomainRow({
  domain,
  selection,
  isExpanded,
  onToggleDomain,
  onToggleCookie,
  onToggleExpand,
}: ExportDomainRowProps) {
  const domainSelection = selection[domain.domain];
  const isAllSelected = domainSelection?.allSelected || false;
  const selectedCount = domainSelection?.cookies.size || 0;
  const isPartial =
    selectedCount > 0 && selectedCount < domain.cookie_count && !isAllSelected;

  return (
    <div>
      <div className="flex items-center gap-2 p-1.5 hover:bg-accent/50 rounded">
        <Checkbox
          checked={isAllSelected || isPartial}
          onCheckedChange={() => onToggleDomain(domain.domain, domain.cookies)}
          className={isPartial ? "opacity-70" : ""}
        />
        <button
          type="button"
          className="flex items-center gap-1 flex-1 text-left text-sm bg-transparent border-none cursor-pointer"
          onClick={() => onToggleExpand(domain.domain)}
        >
          {isExpanded ? (
            <LuChevronDown className="w-3.5 h-3.5" />
          ) : (
            <LuChevronRight className="w-3.5 h-3.5" />
          )}
          <span className="font-medium truncate">{domain.domain}</span>
          <span className="text-xs text-muted-foreground shrink-0">
            ({domain.cookie_count})
          </span>
        </button>
      </div>
      {isExpanded && (
        <div className="ml-7 pl-2 border-l space-y-0.5">
          {domain.cookies.map((cookie) => {
            const isSelected =
              domainSelection?.cookies.has(cookie.name) || false;
            return (
              <div
                key={`${domain.domain}-${cookie.name}`}
                className="flex items-center gap-2 p-1 text-sm hover:bg-accent/30 rounded"
              >
                <Checkbox
                  checked={isSelected || isAllSelected}
                  onCheckedChange={() =>
                    onToggleCookie(
                      domain.domain,
                      cookie.name,
                      domain.cookie_count,
                    )
                  }
                />
                <span className="truncate">{cookie.name}</span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
