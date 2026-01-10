"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
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

type SelectionState = {
  [domain: string]: {
    allSelected: boolean;
    cookies: Set<string>;
  };
};

export function CookieCopyDialog({
  isOpen,
  onClose,
  selectedProfiles,
  profiles,
  runningProfiles,
  onCopyComplete,
}: CookieCopyDialogProps) {
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

  const eligibleSourceProfiles = useMemo(() => {
    return profiles.filter(
      (p) => p.browser === "wayfern" || p.browser === "camoufox",
    );
  }, [profiles]);

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
        count += domainData?.cookie_count || 0;
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
        const current = prev[domain];
        const allSelected = current?.allSelected || false;

        if (allSelected) {
          const newSelection = { ...prev };
          delete newSelection[domain];
          return newSelection;
        } else {
          return {
            ...prev,
            [domain]: {
              allSelected: true,
              cookies: new Set(cookies.map((c) => c.name)),
            },
          };
        }
      });
    },
    [],
  );

  const toggleCookie = useCallback(
    (domain: string, cookieName: string, totalCookies: number) => {
      setSelection((prev) => {
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
      toast.error(
        `Cannot copy cookies: ${runningTargets.map((p) => p.name).join(", ")} ${
          runningTargets.length === 1 ? "is" : "are"
        } still running`,
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
        toast.error(`Some errors occurred: ${errors.join(", ")}`);
      } else {
        toast.success(
          `Successfully copied ${totalCopied + totalReplaced} cookies (${totalReplaced} replaced)`,
        );
        onCopyComplete?.();
        onClose();
      }
    } catch (err) {
      console.error("Failed to copy cookies:", err);
      toast.error(
        `Failed to copy cookies: ${err instanceof Error ? err.message : String(err)}`,
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
            Copy Cookies
          </DialogTitle>
          <DialogDescription>
            Copy cookies from a source profile to {selectedProfiles.length}{" "}
            selected profile{selectedProfiles.length !== 1 ? "s" : ""}.
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto space-y-4">
          <div className="space-y-2">
            <Label>Source Profile</Label>
            <Select
              value={sourceProfileId ?? undefined}
              onValueChange={handleSourceChange}
            >
              <SelectTrigger>
                <SelectValue placeholder="Select a profile to copy cookies from" />
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
                            (running)
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
            <Label>Target Profiles ({targetProfiles.length})</Label>
            <div className="p-2 bg-muted rounded-md max-h-20 overflow-y-auto">
              {targetProfiles.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  {sourceProfileId
                    ? "No other Wayfern/Camoufox profiles selected"
                    : "Select a source profile first"}
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
                          (running)
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
                  Select Cookies{" "}
                  {cookieData && (
                    <span className="text-muted-foreground">
                      ({selectedCookieCount} of {cookieData.total_count}{" "}
                      selected)
                    </span>
                  )}
                </Label>
              </div>

              <div className="relative">
                <LuSearch className="absolute left-2 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
                <Input
                  placeholder="Search domains or cookies..."
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
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
                    ? "No matching cookies found"
                    : "No cookies found"}
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
                Existing cookies with the same name and domain will be replaced.
                Other cookies will be kept.
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
            Cancel
          </RippleButton>
          <LoadingButton
            isLoading={isCopying}
            onClick={() => void handleCopy()}
            disabled={!canCopy}
          >
            Copy {selectedCookieCount > 0 ? `${selectedCookieCount} ` : ""}
            Cookie{selectedCookieCount !== 1 ? "s" : ""}
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
  const domainSelection = selection[domain.domain];
  const isAllSelected = domainSelection?.allSelected || false;
  const selectedCount = domainSelection?.cookies.size || 0;
  const isPartial =
    selectedCount > 0 && selectedCount < domain.cookie_count && !isAllSelected;

  return (
    <div>
      <div className="flex items-center gap-2 p-2 hover:bg-accent/50 rounded">
        <Checkbox
          checked={isAllSelected || isPartial}
          onCheckedChange={() => onToggleDomain(domain.domain, domain.cookies)}
          className={isPartial ? "opacity-70" : ""}
        />
        <button
          type="button"
          className="flex items-center gap-1 flex-1 text-left bg-transparent border-none cursor-pointer"
          onClick={() => onToggleExpand(domain.domain)}
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
