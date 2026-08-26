"use client";

import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "@tauri-apps/plugin-fs";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuChevronRight } from "react-icons/lu";
import { toast } from "sonner";
import { CookiePastePanel, IssueRow } from "@/components/cookie-paste-panel";
import { LoadingButton } from "@/components/loading-button";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
  AnimatedDisclosureChevron,
  AnimatedDisclosureContent,
  AnimatedDisclosureItem,
} from "@/components/ui/animated-disclosure";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { FadingScrollArea } from "@/components/ui/fading-scroll-area";
import { Label } from "@/components/ui/label";
import { RippleButton } from "@/components/ui/ripple";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { translateBackendError } from "@/lib/backend-errors";
import type {
  BrowserProfile,
  CookieAnalysis,
  CookiePasteImportResult,
  CookieReadResult,
  CookieWriteMode,
  DomainCookies,
  UnifiedCookie,
} from "@/types";

interface CookieManagementDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
  initialTab?: "import" | "export";
}

type SelectionState = Record<
  string,
  {
    allSelected: boolean;
    cookies: Set<string>;
  }
>;

/** Long enough that a paste is not re-parsed on every keystroke of a fix. */
const ANALYZE_DEBOUNCE_MS = 250;

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
  const { t } = useTranslation();
  // Import state
  const [pasteContent, setPasteContent] = useState("");
  const [pasteSite, setPasteSite] = useState("");
  const [writeMode, setWriteMode] = useState<CookieWriteMode>("merge");
  const [includeExpired, setIncludeExpired] = useState(false);
  const [analysis, setAnalysis] = useState<CookieAnalysis | null>(null);
  const [isAnalyzing, setIsAnalyzing] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const [isImporting, setIsImporting] = useState(false);
  const [importResult, setImportResult] =
    useState<CookiePasteImportResult | null>(null);

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
        count += domainData?.cookie_count ?? 0;
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
          t("cookies.management.loadFailed", {
            error: translateBackendError(t, err),
          }),
        );
      } finally {
        setIsLoadingExportCookies(false);
      }
    },
    [exportCookieData, t],
  );

  useEffect(() => {
    if (activeTab === "export" && profile && !exportCookieData) {
      void loadExportCookies(profile.id);
    }
  }, [activeTab, profile, exportCookieData, loadExportCookies]);

  const resetImportState = useCallback(() => {
    setPasteContent("");
    setPasteSite("");
    setWriteMode("merge");
    setIncludeExpired(false);
    setAnalysis(null);
    setIsAnalyzing(false);
    setImportError(null);
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

  const profileId = profile?.id;

  useEffect(() => {
    if (!isOpen || !profileId || importResult) return;
    if (pasteContent.trim() === "") {
      setAnalysis(null);
      setIsAnalyzing(false);
      return;
    }

    // Editing the paste is the user acting on the last failure, so retire it.
    setImportError(null);
    setIsAnalyzing(true);
    let cancelled = false;
    const timer = setTimeout(() => {
      void invoke<CookieAnalysis>("analyze_pasted_cookies", {
        profileId,
        content: pasteContent,
        site: pasteSite.trim() === "" ? null : pasteSite,
      })
        .then((result) => {
          if (!cancelled) setAnalysis(result);
        })
        .catch((error: unknown) => {
          if (cancelled) return;
          setAnalysis(null);
          setImportError(translateBackendError(t, error));
        })
        .finally(() => {
          if (!cancelled) setIsAnalyzing(false);
        });
    }, ANALYZE_DEBOUNCE_MS);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [isOpen, profileId, pasteContent, pasteSite, importResult, t]);

  const handleImport = useCallback(async () => {
    if (!profileId) return;
    setIsImporting(true);
    setImportError(null);
    try {
      const result = await invoke<CookiePasteImportResult>(
        "import_pasted_cookies",
        {
          profileId,
          content: pasteContent,
          site: pasteSite.trim() === "" ? null : pasteSite,
          mode: writeMode,
          includeExpired,
        },
      );
      setImportResult(result);
    } catch (error) {
      // Kept inside the dialog rather than toasted: a toast would take the
      // failure away while leaving the user with a paste they cannot fix.
      setImportError(translateBackendError(t, error));
    } finally {
      setIsImporting(false);
    }
  }, [profileId, pasteContent, pasteSite, writeMode, includeExpired, t]);

  const importBlockedReason = useMemo(() => {
    if (pasteContent.trim() === "") return t("cookies.paste.disabledEmpty");
    if (isAnalyzing || !analysis) return null;
    if (analysis.blockedBy) {
      return translateBackendError(t, analysis.blockedBy);
    }
    if (analysis.siteRequired) return t("cookies.paste.disabledSite");
    if (analysis.cookies.length === 0) {
      return t("cookies.paste.disabledNoCookies");
    }
    return null;
  }, [pasteContent, isAnalyzing, analysis, t]);

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
      toast.success(t("cookies.export.success"));
      handleClose();
    } catch (error) {
      toast.error(translateBackendError(t, error));
    } finally {
      setIsExporting(false);
    }
  }, [profile, format, getSelectedCookies, handleClose, t]);

  const toggleDomain = useCallback(
    (domain: string, cookies: UnifiedCookie[]) => {
      setExportSelection((prev) => {
        // `prev[domain]` is `undefined` when the domain was previously fully
        // deselected (entries are deleted on empty — see toggleCookie). Treat
        // missing as "not selected" so re-enabling falls through to the add
        // branch instead of crashing on `.allSelected`.
        if (prev[domain]?.allSelected) {
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
        const current = prev[domain] ?? {
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
      <DialogContent className="max-w-[min(44rem,calc(100%-4rem))]">
        <DialogHeader>
          <DialogTitle>{t("cookies.management.title")}</DialogTitle>
        </DialogHeader>

        <Tabs
          defaultValue={initialTab}
          onValueChange={handleTabChange}
          className="w-full"
        >
          <TabsList className="grid w-full grid-cols-2">
            <TabsTrigger value="import">
              {t("cookies.management.tabImport")}
            </TabsTrigger>
            <TabsTrigger value="export">
              {t("cookies.management.tabExport")}
            </TabsTrigger>
          </TabsList>

          <TabsContent value="import" className="mt-4 space-y-4">
            {!importResult && (
              <>
                <p className="text-sm text-muted-foreground">
                  {t("cookies.management.importDescription")}
                </p>

                <CookiePastePanel
                  content={pasteContent}
                  onContentChange={setPasteContent}
                  site={pasteSite}
                  onSiteChange={setPasteSite}
                  mode={writeMode}
                  onModeChange={setWriteMode}
                  includeExpired={includeExpired}
                  onIncludeExpiredChange={setIncludeExpired}
                  analysis={analysis}
                  isAnalyzing={isAnalyzing}
                  disabled={isImporting}
                />

                <div className="space-y-2">
                  {/* Sits with the button, not at the top of the dialog: the
                      panel is taller than the viewport, so a failure announced
                      above the description is a failure nobody sees. */}
                  {importError && (
                    <Alert variant="destructive">
                      <AlertDescription>{importError}</AlertDescription>
                    </Alert>
                  )}
                  <div className="flex justify-end gap-2">
                    <RippleButton variant="outline" onClick={handleClose}>
                      {t("common.buttons.cancel")}
                    </RippleButton>
                    <LoadingButton
                      isLoading={isImporting}
                      variant={
                        writeMode === "replaceMatchingSites"
                          ? "destructive"
                          : "default"
                      }
                      onClick={() => void handleImport()}
                      disabled={
                        isAnalyzing ||
                        analysis === null ||
                        importBlockedReason !== null
                      }
                    >
                      {t("common.buttons.import")}
                    </LoadingButton>
                  </div>
                  {importBlockedReason && (
                    <p className="text-right text-xs text-muted-foreground">
                      {importBlockedReason}
                    </p>
                  )}
                </div>
              </>
            )}

            {importResult && (
              <div className="space-y-4">
                <div className="grid grid-cols-2 gap-x-4 gap-y-2 rounded-lg bg-muted/30 p-4 sm:grid-cols-4">
                  <ResultCounter
                    label={t("cookies.paste.resultAdded")}
                    value={importResult.added}
                  />
                  <ResultCounter
                    label={t("cookies.paste.resultOverwritten")}
                    value={importResult.overwritten}
                  />
                  <ResultCounter
                    label={t("cookies.paste.resultDeleted")}
                    value={importResult.deleted}
                  />
                  <ResultCounter
                    label={t("cookies.paste.resultSkipped")}
                    value={importResult.skipped}
                  />
                </div>

                {importResult.issues.length > 0 && (
                  <div className="space-y-2">
                    <Label>{t("cookies.paste.issuesTitle")}</Label>
                    <FadingScrollArea className="max-h-[clamp(100px,24vh,300px)]">
                      <div className="space-y-1 pr-3">
                        {importResult.issues.map((issue, index) => (
                          <IssueRow
                            key={`${issue.code}-${issue.source ?? ""}-${index}`}
                            issue={issue}
                          />
                        ))}
                      </div>
                    </FadingScrollArea>
                  </div>
                )}

                <div className="flex justify-end">
                  <RippleButton onClick={handleClose}>
                    {t("cookies.management.doneButton")}
                  </RippleButton>
                </div>
              </div>
            )}
          </TabsContent>

          <TabsContent value="export" className="mt-4 space-y-3">
            <div className="space-y-2">
              <Label>{t("cookies.export.formatLabel")}</Label>
              <Select
                value={format}
                onValueChange={(v) => {
                  setFormat(v as "netscape" | "json");
                }}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="json">
                    {t("cookies.export.json")}
                  </SelectItem>
                  <SelectItem value="netscape">
                    {t("cookies.export.netscape")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <Label>
                  {t("cookies.management.cookiesLabel")}{" "}
                  {exportCookieData && (
                    <span className="font-normal text-muted-foreground">
                      {t("cookies.management.selectionStatus", {
                        selected: selectedExportCount,
                        total: exportCookieData.total_count,
                      })}
                    </span>
                  )}
                </Label>
                {exportCookieData && exportCookieData.total_count > 0 && (
                  <button
                    type="button"
                    className="text-xs text-muted-foreground transition-colors hover:text-foreground"
                    onClick={toggleSelectAll}
                  >
                    {selectedExportCount === exportCookieData.total_count
                      ? t("cookies.management.deselectAll")
                      : t("cookies.management.selectAll")}
                  </button>
                )}
              </div>

              {isLoadingExportCookies ? (
                <div className="flex h-24 items-center justify-center">
                  <div className="size-5 animate-spin rounded-full border-2 border-primary border-t-transparent" />
                </div>
              ) : !exportCookieData || exportCookieData.domains.length === 0 ? (
                <div className="rounded-md border p-4 text-center text-sm text-muted-foreground">
                  {t("cookies.management.noCookies")}
                </div>
              ) : (
                <FadingScrollArea className="h-[clamp(140px,30vh,420px)]">
                  <div className="space-y-1 p-2">
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
                </FadingScrollArea>
              )}
            </div>

            <div className="flex justify-end gap-2">
              <RippleButton variant="outline" onClick={handleClose}>
                {t("common.buttons.cancel")}
              </RippleButton>
              <LoadingButton
                isLoading={isExporting}
                onClick={() => void handleExport()}
                disabled={selectedExportCount === 0}
              >
                {t("cookies.management.exportButton")}
              </LoadingButton>
            </div>
          </TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}

/** Zeros are shown too: "deleted 0" is the reassurance replace mode needs. */
function ResultCounter({ label, value }: { label: string; value: number }) {
  return (
    <div className="space-y-0.5">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="text-lg font-medium tabular-nums">{value}</div>
    </div>
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
  const isAllSelected = domainSelection?.allSelected ?? false;
  const selectedCount = domainSelection?.cookies.size ?? 0;
  const isPartial =
    selectedCount > 0 && selectedCount < domain.cookie_count && !isAllSelected;

  return (
    <AnimatedDisclosureItem>
      <div className="flex items-center gap-2 rounded p-1.5 hover:bg-muted">
        <Checkbox
          checked={isAllSelected || isPartial}
          onCheckedChange={() => {
            onToggleDomain(domain.domain, domain.cookies);
          }}
          className={isPartial ? "opacity-70" : ""}
        />
        <button
          type="button"
          aria-expanded={isExpanded}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-1 border-none bg-transparent text-left text-sm"
          onClick={() => {
            onToggleExpand(domain.domain);
          }}
        >
          <AnimatedDisclosureChevron open={isExpanded}>
            <LuChevronRight className="size-3.5" />
          </AnimatedDisclosureChevron>
          <span className="truncate font-medium">{domain.domain}</span>
          <span className="shrink-0 text-xs text-muted-foreground">
            ({domain.cookie_count})
          </span>
        </button>
      </div>
      <AnimatedDisclosureContent
        open={isExpanded}
        className="ml-7 space-y-0.5 border-l pl-2"
      >
        {domain.cookies.map((cookie) => {
          const isSelected = domainSelection?.cookies.has(cookie.name) ?? false;
          return (
            <div
              key={`${domain.domain}-${cookie.name}`}
              className="flex items-center gap-2 rounded p-1 text-sm hover:bg-accent/30"
            >
              <Checkbox
                checked={isSelected || isAllSelected}
                onCheckedChange={() => {
                  onToggleCookie(
                    domain.domain,
                    cookie.name,
                    domain.cookie_count,
                  );
                }}
              />
              <span className="truncate">{cookie.name}</span>
            </div>
          );
        })}
      </AnimatedDisclosureContent>
    </AnimatedDisclosureItem>
  );
}
