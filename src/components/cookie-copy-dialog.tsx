"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  LuChevronDown,
  LuChevronRight,
  LuCookie,
  LuSearch,
} from "react-icons/lu";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { getBrowserIcon } from "@/lib/browser-utils";
import type {
  BrowserProfile,
  CookieCopyRequest,
  CookieCopyResult,
  CookieReadResult,
  DomainCookies,
  SelectedCookie,
  UnifiedCookie,
} from "@/types";
import { RippleButton } from "./ui/ripple";

interface CookieCopyDialogProps {
  isOpen: boolean;
  onClose: () => void;
  selectedProfiles: string[];
  profiles: BrowserProfile[];
  runningProfiles: Set<string>;
  onCopyComplete?: () => void;
}

type SelectionState = Record<
  string,
  {
    allSelected: boolean;
    cookies: Set<string>;
  }
>;

export function CookieCopyDialog({
  isOpen,
  onClose,
  selectedProfiles,
  profiles,
  runningProfiles,
  onCopyComplete,
}: CookieCopyDialogProps) {
  const { t } = useTranslation();
  const [sourceProfileId, setSourceProfileId] = useState<string | null>(null);
  const [cookieData, setCookieData] = useState<CookieReadResult | null>(null);
  const [isLoadingCookies, setIsLoadingCookies] = useState(false);
  const [isCopying, setIsCopying] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [selection, setSelection] = useState<SelectionState>({});
  const [expandedDomains, setExpandedDomains] = useState<Set<string>>(
    new Set(),
  );
  const [error, setError] = useState<string | null>(null);

  // Never offer a selected profile as a source — you can't copy a profile's
  // cookies onto itself, and including it here would leave the user in a
  // dead-end state (source picked = target list empty = copy button disabled).
  const eligibleSourceProfiles = useMemo(() => {
    return profiles.filter(
      (p) =>
        !selectedProfiles.includes(p.id) &&
        (p.browser === "wayfern" || p.browser === "camoufox"),
    );
  }, [profiles, selectedProfiles]);

  const targetProfiles = useMemo(() => {
    return profiles.filter(
      (p) =>
        selectedProfiles.includes(p.id) &&
        p.id !== sourceProfileId &&
        (p.browser === "wayfern" || p.browser === "camoufox"),
    );
  }, [profiles, selectedProfiles, sourceProfileId]);

  const filteredDomains = useMemo(() => {
    if (!cookieData) return [];
    if (!searchQuery.trim()) return cookieData.domains;

    const query = searchQuery.toLowerCase();
    return cookieData.domains.filter(
      (d) =>
        d.domain.toLowerCase().includes(query) ||
        d.cookies.some((c) => c.name.toLowerCase().includes(query)),
    );
  }, [cookieData, searchQuery]);

  const selectedCookieCount = useMemo(() => {
    let count = 0;
    for (const domain of Object.keys(selection)) {
      const domainSelection = selection[domain];
      if (domainSelection.allSelected) {
        const domainData = cookieData?.domains.find((d) => d.domain === domain);
        count += domainData?.cookie_count ?? 0;
      } else {
        count += domainSelection.cookies.size;
      }
    }
    return count;
  }, [selection, cookieData]);

  const loadCookies = useCallback(async (profileId: string) => {
    setIsLoadingCookies(true);
    setError(null);
    setCookieData(null);
    setSelection({});

    try {
      const result = await invoke<CookieReadResult>("read_profile_cookies", {
        profileId,
      });
      setCookieData(result);
    } catch (err) {
      console.error("Failed to load cookies:", err);
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsLoadingCookies(false);
    }
  }, []);

  const handleSourceChange = useCallback(
    (profileId: string) => {
      setSourceProfileId(profileId);
      void loadCookies(profileId);
    },
    [loadCookies],
  );

  const toggleDomain = useCallback(
    (domain: string, cookies: UnifiedCookie[]) => {
      setSelection((prev) => {
        // `prev[domain]` is `undefined` for any domain not yet interacted with
        // and after the user fully deselects it (toggleCookie deletes the
        // entry on empty). Treat missing as "not selected".
        if (prev[domain]?.allSelected) {
          const newSelection = { ...prev };
          delete newSelection[domain];
          return newSelection;
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
      setSelection((prev) => {
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

        const allSelected = newCookies.size === totalCookies;

        if (newCookies.size === 0) {
          const newSelection = { ...prev };
          delete newSelection[domain];
          return newSelection;
        }

        return {
          ...prev,
          [domain]: {
            allSelected,
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

  const buildSelectedCookies = useCallback((): SelectedCookie[] => {
    const result: SelectedCookie[] = [];

    for (const [domain, domainSelection] of Object.entries(selection)) {
      if (domainSelection.allSelected) {
        result.push({ domain, name: "" });
      } else {
        for (const cookieName of domainSelection.cookies) {
          result.push({ domain, name: cookieName });
        }
      }
    }

    return result;
  }, [selection]);

  const handleCopy = useCallback(async () => {
    if (!sourceProfileId || targetProfiles.length === 0) return;

    const runningTargets = targetProfiles.filter((p) =>
      runningProfiles.has(p.id),
    );
    if (runningTargets.length > 0) {
      const names = runningTargets.map((p) => p.name).join(", ");
      toast.error(
        runningTargets.length === 1
          ? t("cookies.copy.cannotCopyRunningOne", { names })
          : t("cookies.copy.cannotCopyRunningMany", { names }),
      );
      return;
    }

    setIsCopying(true);
    setError(null);

    try {
      const selectedCookies = buildSelectedCookies();
      const request: CookieCopyRequest = {
        source_profile_id: sourceProfileId,
        target_profile_ids: targetProfiles.map((p) => p.id),
        selected_cookies: selectedCookies,
      };

      const results = await invoke<CookieCopyResult[]>("copy_profile_cookies", {
        request,
      });

      let totalCopied = 0;
      let totalReplaced = 0;
      const errors: string[] = [];

      for (const result of results) {
        totalCopied += result.cookies_copied;
        totalReplaced += result.cookies_replaced;
        errors.push(...result.errors);
      }

      if (errors.length > 0) {
        toast.error(
          t("cookies.copy.someErrors", { errors: errors.join(", ") }),
        );
      } else {
        toast.success(
          t("cookies.copy.successMessage", {
            copied: totalCopied + totalReplaced,
            replaced: totalReplaced,
          }),
        );
        onCopyComplete?.();
        onClose();
      }
    } catch (err) {
      console.error("Failed to copy cookies:", err);
      toast.error(
        t("cookies.copy.failedMessage", {
          error: err instanceof Error ? err.message : String(err),
        }),
      );
    } finally {
      setIsCopying(false);
    }
  }, [
    sourceProfileId,
    targetProfiles,
    runningProfiles,
    buildSelectedCookies,
    onCopyComplete,
    onClose,
    t,
  ]);

  useEffect(() => {
    if (isOpen) {
      setSourceProfileId(null);
      setCookieData(null);
      setSelection({});
      setSearchQuery("");
      setExpandedDomains(new Set());
      setError(null);
    }
  }, [isOpen]);

  const canCopy =
    sourceProfileId &&
    targetProfiles.length > 0 &&
    selectedCookieCount > 0 &&
    !isCopying;

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-2xl max-h-[80vh] flex flex-col">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <LuCookie className="w-5 h-5" />
            {t("cookies.copy.title")}
          </DialogTitle>
          <DialogDescription>
            {selectedProfiles.length === 1
              ? t("cookies.copy.dialogDescription_one", {
                  count: selectedProfiles.length,
                })
              : t("cookies.copy.dialogDescription_other", {
                  count: selectedProfiles.length,
                })}
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto space-y-4">
          <div className="space-y-2">
            <Label>{t("cookies.copy.sourceProfile")}</Label>
            <Select
              value={sourceProfileId ?? undefined}
              onValueChange={handleSourceChange}
            >
              <SelectTrigger>
                <SelectValue
                  placeholder={t("cookies.copy.sourcePlaceholder")}
                />
              </SelectTrigger>
              <SelectContent>
                {eligibleSourceProfiles.map((profile) => {
                  const IconComponent = getBrowserIcon(profile.browser);
                  const isRunning = runningProfiles.has(profile.id);
                  return (
                    <SelectItem
                      key={profile.id}
                      value={profile.id}
                      disabled={isRunning}
                    >
                      <div className="flex items-center gap-2">
                        {IconComponent && <IconComponent className="w-4 h-4" />}
                        <span>{profile.name}</span>
                        {isRunning && (
                          <span className="text-xs text-muted-foreground">
                            {t("cookies.copy.running")}
                          </span>
                        )}
                      </div>
                    </SelectItem>
                  );
                })}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-2">
            <Label>
              {t("cookies.copy.targetProfiles", {
                count: targetProfiles.length,
              })}
            </Label>
            <div className="p-2 bg-muted rounded-md max-h-20 overflow-y-auto">
              {targetProfiles.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  {sourceProfileId
                    ? t("cookies.copy.noOtherTargets")
                    : t("cookies.copy.selectSourceFirst")}
                </p>
              ) : (
                <div className="flex flex-wrap gap-1">
                  {targetProfiles.map((p) => (
                    <span
                      key={p.id}
                      className="inline-flex items-center gap-1 px-2 py-0.5 bg-background rounded text-sm"
                    >
                      {p.name}
                      {runningProfiles.has(p.id) && (
                        <span className="text-xs text-destructive">
                          {t("cookies.copy.running")}
                        </span>
                      )}
                    </span>
                  ))}
                </div>
              )}
            </div>
          </div>

          {sourceProfileId && (
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <Label>
                  {t("cookies.copy.selectCookies")}{" "}
                  {cookieData && (
                    <span className="text-muted-foreground">
                      {t("cookies.copy.selectionStatus", {
                        selected: selectedCookieCount,
                        total: cookieData.total_count,
                      })}
                    </span>
                  )}
                </Label>
              </div>

              <div className="relative">
                <LuSearch className="absolute left-2 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
                <Input
                  placeholder={t("cookies.copy.searchPlaceholder")}
                  value={searchQuery}
                  onChange={(e) => {
                    setSearchQuery(e.target.value);
                  }}
                  className="pl-8"
                />
              </div>

              {isLoadingCookies ? (
                <div className="flex items-center justify-center h-40">
                  <div className="animate-spin h-6 w-6 border-2 border-primary border-t-transparent rounded-full" />
                </div>
              ) : error ? (
                <div className="p-4 text-center text-destructive bg-destructive/10 rounded-md">
                  {error}
                </div>
              ) : filteredDomains.length === 0 ? (
                <div className="p-4 text-center text-muted-foreground">
                  {searchQuery
                    ? t("cookies.copy.noMatching")
                    : t("cookies.copy.noFound")}
                </div>
              ) : (
                <ScrollArea className="h-[250px] border rounded-md">
                  <div className="p-2 space-y-1">
                    {filteredDomains.map((domain) => (
                      <DomainRow
                        key={domain.domain}
                        domain={domain}
                        selection={selection}
                        isExpanded={expandedDomains.has(domain.domain)}
                        onToggleDomain={toggleDomain}
                        onToggleCookie={toggleCookie}
                        onToggleExpand={toggleExpand}
                      />
                    ))}
                  </div>
                </ScrollArea>
              )}

              <p className="text-xs text-muted-foreground">
                {t("cookies.copy.replaceNote")}
              </p>
            </div>
          )}
        </div>

        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={onClose}
            disabled={isCopying}
          >
            {t("common.buttons.cancel")}
          </RippleButton>
          <LoadingButton
            isLoading={isCopying}
            onClick={() => void handleCopy()}
            disabled={!canCopy}
          >
            {selectedCookieCount === 0
              ? t("cookies.copy.copyButtonEmpty")
              : selectedCookieCount === 1
                ? t("cookies.copy.copyButton_one", {
                    count: selectedCookieCount,
                  })
                : t("cookies.copy.copyButton_other", {
                    count: selectedCookieCount,
                  })}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface DomainRowProps {
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

function DomainRow({
  domain,
  selection,
  isExpanded,
  onToggleDomain,
  onToggleCookie,
  onToggleExpand,
}: DomainRowProps) {
  // `selection[domain.domain]` is `undefined` for domains the user hasn't
  // touched yet (initial state after loading cookies is `{}`) and for any
  // domain the user fully deselected (toggleCookie deletes the entry on
  // empty). Default to "no cookies selected" instead of crashing.
  const domainSelection = selection[domain.domain];
  const isAllSelected = domainSelection?.allSelected ?? false;
  const selectedCount = domainSelection?.cookies.size ?? 0;
  const isPartial =
    selectedCount > 0 && selectedCount < domain.cookie_count && !isAllSelected;

  return (
    <div>
      <div className="flex items-center gap-2 p-2 hover:bg-accent/50 rounded">
        <Checkbox
          checked={isAllSelected || isPartial}
          onCheckedChange={() => {
            onToggleDomain(domain.domain, domain.cookies);
          }}
          className={isPartial ? "opacity-70" : ""}
        />
        <button
          type="button"
          className="flex items-center gap-1 flex-1 text-left bg-transparent border-none cursor-pointer"
          onClick={() => {
            onToggleExpand(domain.domain);
          }}
        >
          {isExpanded ? (
            <LuChevronDown className="w-4 h-4" />
          ) : (
            <LuChevronRight className="w-4 h-4" />
          )}
          <span className="font-medium">{domain.domain}</span>
          <span className="text-xs text-muted-foreground">
            ({domain.cookie_count})
          </span>
        </button>
      </div>
      {isExpanded && (
        <div className="ml-8 pl-2 border-l space-y-1">
          {domain.cookies.map((cookie) => {
            const isSelected =
              domainSelection?.cookies.has(cookie.name) ?? false;
            return (
              <div
                key={`${domain.domain}-${cookie.name}`}
                className="flex items-center gap-2 p-1 text-sm hover:bg-accent/30 rounded"
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
        </div>
      )}
    </div>
  );
}
