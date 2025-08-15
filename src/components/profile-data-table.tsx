"use client";

import {
  type ColumnDef,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  type SortingState,
  useReactTable,
} from "@tanstack/react-table";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import * as React from "react";
import { CiCircleCheck } from "react-icons/ci";
import { IoEllipsisHorizontal } from "react-icons/io5";
import { LuCheck, LuChevronDown, LuChevronUp } from "react-icons/lu";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useBrowserState } from "@/hooks/use-browser-state";
import { useTableSorting } from "@/hooks/use-table-sorting";
import {
  getBrowserDisplayName,
  getBrowserIcon,
  getCurrentOS,
} from "@/lib/browser-utils";
import { trimName } from "@/lib/name-utils";
import { cn } from "@/lib/utils";
import type { BrowserProfile, StoredProxy } from "@/types";
import { LoadingButton } from "./loading-button";
import MultipleSelector, { type Option } from "./multiple-selector";
import { Input } from "./ui/input";
import { RippleButton } from "./ui/ripple";

const TagsCell = React.memo<{
  profile: BrowserProfile;
  isDisabled: boolean;
  tagsOverrides: Record<string, string[]>;
  allTags: string[];
  setAllTags: React.Dispatch<React.SetStateAction<string[]>>;
  openTagsEditorFor: string | null;
  setOpenTagsEditorFor: React.Dispatch<React.SetStateAction<string | null>>;
  setTagsOverrides: React.Dispatch<
    React.SetStateAction<Record<string, string[]>>
  >;
}>(
  ({
    profile,
    isDisabled,
    tagsOverrides,
    allTags,
    setAllTags,
    openTagsEditorFor,
    setOpenTagsEditorFor,
    setTagsOverrides,
  }) => {
    const effectiveTags: string[] = Object.hasOwn(tagsOverrides, profile.name)
      ? tagsOverrides[profile.name]
      : (profile.tags ?? []);

    const valueOptions: Option[] = React.useMemo(
      () => effectiveTags.map((t) => ({ value: t, label: t })),
      [effectiveTags],
    );
    const allOptions: Option[] = React.useMemo(
      () => allTags.map((t) => ({ value: t, label: t })),
      [allTags],
    );

    const handleChange = React.useCallback(
      async (opts: Option[]) => {
        const newTagsRaw = opts.map((o) => o.value);
        // Dedupe while preserving order
        const seen = new Set<string>();
        const newTags: string[] = [];
        for (const t of newTagsRaw) {
          if (!seen.has(t)) {
            seen.add(t);
            newTags.push(t);
          }
        }
        setTagsOverrides((prev) => ({ ...prev, [profile.name]: newTags }));
        try {
          await invoke<BrowserProfile>("update_profile_tags", {
            profileName: profile.name,
            tags: newTags,
          });
          setAllTags((prev) => {
            const next = new Set(prev);
            for (const t of newTags) next.add(t);
            return Array.from(next).sort();
          });
        } catch (error) {
          console.error("Failed to update tags:", error);
        }
      },
      [profile.name, setAllTags, setTagsOverrides],
    );

    const containerRef = React.useRef<HTMLDivElement | null>(null);
    const editorRef = React.useRef<HTMLDivElement | null>(null);
    const [visibleCount, setVisibleCount] = React.useState<number>(
      effectiveTags.length,
    );

    React.useLayoutEffect(() => {
      // Only measure when not editing this profile's tags
      if (openTagsEditorFor === profile.name) return;
      const container = containerRef.current;
      if (!container) return;

      let timeoutId: number | undefined;
      const compute = () => {
        if (timeoutId) clearTimeout(timeoutId);
        timeoutId = window.setTimeout(() => {
          const available = container.clientWidth;
          if (available <= 0) return;
          const canvas = document.createElement("canvas");
          const ctx = canvas.getContext("2d");
          if (!ctx) return;
          const style = window.getComputedStyle(container);
          const font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
          ctx.font = font;
          const padding = 16;
          const gap = 4;
          let used = 0;
          let count = 0;
          for (let i = 0; i < effectiveTags.length; i++) {
            const text = effectiveTags[i];
            const width = Math.ceil(ctx.measureText(text).width) + padding;
            const remaining = effectiveTags.length - (i + 1);
            let extra = 0;
            if (remaining > 0) {
              const plusText = `+${remaining}`;
              extra = Math.ceil(ctx.measureText(plusText).width) + padding;
            }
            const nextUsed =
              used +
              (used > 0 ? gap : 0) +
              width +
              (remaining > 0 ? gap + extra : 0);
            if (nextUsed <= available) {
              used += (used > 0 ? gap : 0) + width;
              count = i + 1;
            } else {
              break;
            }
          }
          setVisibleCount(count);
        }, 16); // Debounce with RAF timing
      };
      compute();
      const ro = new ResizeObserver(compute);
      ro.observe(container);
      return () => {
        ro.disconnect();
        if (timeoutId) clearTimeout(timeoutId);
      };
    }, [effectiveTags, openTagsEditorFor, profile.name]);

    React.useEffect(() => {
      if (openTagsEditorFor !== profile.name) return;
      const handleClick = (e: MouseEvent) => {
        const target = e.target as Node | null;
        if (
          editorRef.current &&
          target &&
          !editorRef.current.contains(target)
        ) {
          setOpenTagsEditorFor(null);
        }
      };
      document.addEventListener("mousedown", handleClick);
      return () => document.removeEventListener("mousedown", handleClick);
    }, [openTagsEditorFor, profile.name, setOpenTagsEditorFor]);

    React.useEffect(() => {
      if (openTagsEditorFor === profile.name && editorRef.current) {
        // Focus the inner input of MultipleSelector on open
        const inputEl = editorRef.current.querySelector("input");
        if (inputEl) {
          (inputEl as HTMLInputElement).focus();
        }
      }
    }, [openTagsEditorFor, profile.name]);

    if (openTagsEditorFor !== profile.name) {
      const hiddenCount = Math.max(0, effectiveTags.length - visibleCount);
      return (
        <div className="w-48 h-full cursor-pointer">
          <button
            type="button"
            ref={containerRef as unknown as React.RefObject<HTMLButtonElement>}
            className={cn(
              "flex items-center gap-1 overflow-hidden cursor-pointer bg-transparent border-none p-1 w-full h-full",
              isDisabled && "opacity-60",
            )}
            onClick={() => {
              if (!isDisabled) setOpenTagsEditorFor(profile.name);
            }}
          >
            {effectiveTags.slice(0, visibleCount).map((t) => (
              <Badge key={t} variant="secondary" className="px-2 py-0 text-xs">
                {t}
              </Badge>
            ))}
            {effectiveTags.length === 0 && (
              <span className="text-muted-foreground">No tags</span>
            )}
            {hiddenCount > 0 && (
              <Badge variant="outline" className="px-2 py-0 text-xs">
                +{hiddenCount}
              </Badge>
            )}
          </button>
        </div>
      );
    }

    return (
      <div
        className={cn("w-48", isDisabled && "opacity-60 pointer-events-none")}
      >
        <div ref={editorRef}>
          <MultipleSelector
            value={valueOptions}
            options={allOptions}
            onChange={(opts) => void handleChange(opts)}
            creatable
            selectFirstItem={false}
            placeholder={effectiveTags.length === 0 ? "Add tags" : ""}
            className="overflow-x-hidden overflow-y-scroll h-7 bg-transparent"
            badgeClassName=""
            inputProps={{
              className: "py-1",
              onKeyDown: (e) => {
                if (e.key === "Escape") setOpenTagsEditorFor(null);
              },
            }}
          />
        </div>
      </div>
    );
  },
);

TagsCell.displayName = "TagsCell";

interface ProfilesDataTableProps {
  data: BrowserProfile[];
  onLaunchProfile: (profile: BrowserProfile) => void | Promise<void>;
  onKillProfile: (profile: BrowserProfile) => void | Promise<void>;
  onDeleteProfile: (profile: BrowserProfile) => void | Promise<void>;
  onRenameProfile: (oldName: string, newName: string) => Promise<void>;
  onConfigureCamoufox?: (profile: BrowserProfile) => void;
  runningProfiles: Set<string>;
  isUpdating: (browser: string) => boolean;
  onDeleteSelectedProfiles?: (profileNames: string[]) => Promise<void>;
  onAssignProfilesToGroup?: (profileNames: string[]) => void;
  selectedGroupId?: string | null;
  selectedProfiles?: string[];
  onSelectedProfilesChange?: (profiles: string[]) => void;
}

interface TableMeta {
  tagsOverrides: Record<string, string[]>;
  allTags: string[];
  setAllTags: React.Dispatch<React.SetStateAction<string[]>>;
  openTagsEditorFor: string | null;
  setOpenTagsEditorFor: React.Dispatch<React.SetStateAction<string | null>>;
  setTagsOverrides: React.Dispatch<
    React.SetStateAction<Record<string, string[]>>
  >;
  selectedProfiles: Set<string>;
  showCheckboxes: boolean;
  browserState: {
    isClient: boolean;
    canLaunchProfile: (profile: BrowserProfile) => boolean;
    canSelectProfile: (profile: BrowserProfile) => boolean;
    getLaunchTooltipContent: (profile: BrowserProfile) => string | null;
  };
  runningProfiles: Set<string>;
  launchingProfiles: Set<string>;
  stoppingProfiles: Set<string>;
  isUpdating: (browser: string) => boolean;
  proxyOverrides: Record<string, string | null>;
  storedProxies: StoredProxy[];
  openProxySelectorFor: string | null;
  profileToRename: BrowserProfile | null;
  newProfileName: string;
  renameError: string | null;
  isRenamingSaving: boolean;
  onLaunchProfile: (profile: BrowserProfile) => void | Promise<void>;
  onKillProfile: (profile: BrowserProfile) => void | Promise<void>;
  onConfigureCamoufox?: (profile: BrowserProfile) => void;
  onAssignProfilesToGroup?: (profileNames: string[]) => void;
}

export function ProfilesDataTable({
  data,
  onLaunchProfile,
  onKillProfile,
  onDeleteProfile,
  onRenameProfile,
  onConfigureCamoufox,
  runningProfiles,
  isUpdating,
  onAssignProfilesToGroup,
  selectedGroupId,
  selectedProfiles: externalSelectedProfiles = [],
  onSelectedProfilesChange,
}: ProfilesDataTableProps) {
  const { getTableSorting, updateSorting, isLoaded } = useTableSorting();
  const [sorting, setSorting] = React.useState<SortingState>([]);
  const [profileToRename, setProfileToRename] =
    React.useState<BrowserProfile | null>(null);
  const [newProfileName, setNewProfileName] = React.useState("");
  const [renameError, setRenameError] = React.useState<string | null>(null);
  const [isRenamingSaving, setIsRenamingSaving] = React.useState(false);
  const renameContainerRef = React.useRef<HTMLDivElement | null>(null);
  const [profileToDelete, setProfileToDelete] =
    React.useState<BrowserProfile | null>(null);
  const [isDeleting, setIsDeleting] = React.useState(false);
  const [launchingProfiles, setLaunchingProfiles] = React.useState<Set<string>>(
    new Set(),
  );
  const [stoppingProfiles, setStoppingProfiles] = React.useState<Set<string>>(
    new Set(),
  );

  const [storedProxies, setStoredProxies] = React.useState<StoredProxy[]>([]);
  const [openProxySelectorFor, setOpenProxySelectorFor] = React.useState<
    string | null
  >(null);
  const [proxyOverrides, setProxyOverrides] = React.useState<
    Record<string, string | null>
  >({});
  const [selectedProfiles, setSelectedProfiles] = React.useState<Set<string>>(
    new Set(externalSelectedProfiles),
  );
  const [showCheckboxes, setShowCheckboxes] = React.useState(false);
  const [tagsOverrides, setTagsOverrides] = React.useState<
    Record<string, string[]>
  >({});
  const [allTags, setAllTags] = React.useState<string[]>([]);
  const [openTagsEditorFor, setOpenTagsEditorFor] = React.useState<
    string | null
  >(null);

  const loadAllTags = React.useCallback(async () => {
    try {
      const tags = await invoke<string[]>("get_all_tags");
      setAllTags(tags);
    } catch (error) {
      console.error("Failed to load tags:", error);
    }
  }, []);

  const handleProxySelection = React.useCallback(
    async (profileName: string, proxyId: string | null) => {
      try {
        await invoke("update_profile_proxy", {
          profileName,
          proxyId,
        });
        setProxyOverrides((prev) => ({ ...prev, [profileName]: proxyId }));
        // Notify other parts of the app so usage counts and lists refresh
        await emit("profile-updated");
      } catch (error) {
        console.error("Failed to update proxy settings:", error);
      } finally {
        setOpenProxySelectorFor(null);
      }
    },
    [],
  );

  // Filter data by selected group
  const filteredData = React.useMemo(() => {
    if (!selectedGroupId || selectedGroupId === "default") {
      return data.filter((profile) => !profile.group_id);
    }

    return data.filter((profile) => profile.group_id === selectedGroupId);
  }, [data, selectedGroupId]);

  // Use shared browser state hook
  const browserState = useBrowserState(
    filteredData,
    runningProfiles,
    isUpdating,
    launchingProfiles,
    stoppingProfiles,
  );

  // Load stored proxies
  const loadStoredProxies = React.useCallback(async () => {
    try {
      const proxiesList = await invoke<StoredProxy[]>("get_stored_proxies");
      setStoredProxies(proxiesList);
    } catch (error) {
      console.error("Failed to load stored proxies:", error);
    }
  }, []);

  React.useEffect(() => {
    if (browserState.isClient) {
      void loadStoredProxies();
    }
  }, [browserState.isClient, loadStoredProxies]);

  // Keep stored proxies up-to-date by listening for changes emitted elsewhere in the app
  React.useEffect(() => {
    if (!browserState.isClient) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        unlisten = await listen("stored-proxies-changed", () => {
          void loadStoredProxies();
        });
        // Also refresh tags on profile updates
        await listen("profile-updated", () => {
          void loadAllTags();
        });
      } catch (_err) {
        // Best-effort only
      }
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, [browserState.isClient, loadStoredProxies, loadAllTags]);

  // Automatically deselect profiles that become running, updating, launching, or stopping
  React.useEffect(() => {
    setSelectedProfiles((prev) => {
      const newSet = new Set(prev);
      let hasChanges = false;

      for (const profileName of prev) {
        const profile = filteredData.find((p) => p.name === profileName);
        if (profile) {
          const isRunning =
            browserState.isClient && runningProfiles.has(profile.name);
          const isLaunching = launchingProfiles.has(profile.name);
          const isStopping = stoppingProfiles.has(profile.name);
          const isBrowserUpdating = isUpdating(profile.browser);

          if (isRunning || isLaunching || isStopping || isBrowserUpdating) {
            newSet.delete(profileName);
            hasChanges = true;
          }
        }
      }

      if (hasChanges) {
        onSelectedProfilesChange?.(Array.from(newSet));
        return newSet;
      }

      return prev;
    });
  }, [
    filteredData,
    runningProfiles,
    launchingProfiles,
    stoppingProfiles,
    isUpdating,
    browserState.isClient,
    onSelectedProfilesChange,
  ]);

  // Sync external selected profiles with internal state
  React.useEffect(() => {
    const newSet = new Set(externalSelectedProfiles);
    setSelectedProfiles(newSet);
    setShowCheckboxes(newSet.size > 0);
  }, [externalSelectedProfiles]);

  // Update local sorting state when settings are loaded
  React.useEffect(() => {
    if (isLoaded && browserState.isClient) {
      setSorting(getTableSorting());
    }
  }, [isLoaded, getTableSorting, browserState.isClient]);

  // Handle sorting changes
  const handleSortingChange = React.useCallback(
    (updater: React.SetStateAction<SortingState>) => {
      if (!browserState.isClient) return;
      const newSorting =
        typeof updater === "function" ? updater(sorting) : updater;
      setSorting(newSorting);
      updateSorting(newSorting);
    },
    [browserState.isClient, sorting, updateSorting],
  );

  const handleRename = React.useCallback(async () => {
    if (!profileToRename || !newProfileName.trim()) return;

    try {
      setIsRenamingSaving(true);
      await onRenameProfile(profileToRename.name, newProfileName.trim());
      setProfileToRename(null);
      setNewProfileName("");
      setRenameError(null);
    } catch (error) {
      setRenameError(
        error instanceof Error ? error.message : "Failed to rename profile",
      );
    } finally {
      setIsRenamingSaving(false);
    }
  }, [profileToRename, newProfileName, onRenameProfile]);

  // Cancel inline rename on outside click
  React.useEffect(() => {
    if (!profileToRename) return;
    const handleClickOutside = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (
        target &&
        renameContainerRef.current &&
        !renameContainerRef.current.contains(target)
      ) {
        setProfileToRename(null);
        setNewProfileName("");
        setRenameError(null);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [profileToRename]);

  const handleDelete = async () => {
    if (!profileToDelete) return;

    setIsDeleting(true);
    try {
      await onDeleteProfile(profileToDelete);
      setProfileToDelete(null);
    } catch (error) {
      console.error("Failed to delete profile:", error);
    } finally {
      setIsDeleting(false);
    }
  };

  // Handle icon/checkbox click
  const handleIconClick = React.useCallback(
    (profileName: string) => {
      const profile = filteredData.find((p) => p.name === profileName);
      if (!profile) return;

      // Prevent selection of profiles whose browsers are updating
      if (!browserState.canSelectProfile(profile)) {
        return;
      }

      setShowCheckboxes(true);
      setSelectedProfiles((prev) => {
        const newSet = new Set(prev);
        if (newSet.has(profileName)) {
          newSet.delete(profileName);
        } else {
          newSet.add(profileName);
        }

        // Hide checkboxes if no profiles are selected
        if (newSet.size === 0) {
          setShowCheckboxes(false);
        }

        // Notify parent component
        if (onSelectedProfilesChange) {
          onSelectedProfilesChange(Array.from(newSet));
        }

        return newSet;
      });
    },
    [filteredData, browserState.canSelectProfile, onSelectedProfilesChange],
  );

  React.useEffect(() => {
    if (browserState.isClient) {
      void loadAllTags();
    }
  }, [browserState.isClient, loadAllTags]);

  // Handle checkbox change
  const handleCheckboxChange = React.useCallback(
    (profileName: string, checked: boolean) => {
      setSelectedProfiles((prev) => {
        const newSet = new Set(prev);
        if (checked) {
          newSet.add(profileName);
        } else {
          newSet.delete(profileName);
        }

        // Hide checkboxes if no profiles are selected
        if (newSet.size === 0) {
          setShowCheckboxes(false);
        }

        // Notify parent component
        if (onSelectedProfilesChange) {
          onSelectedProfilesChange(Array.from(newSet));
        }

        return newSet;
      });
    },
    [onSelectedProfilesChange],
  );

  // Handle select all checkbox
  const handleToggleAll = React.useCallback(
    (checked: boolean) => {
      const newSet = checked
        ? new Set(
            filteredData
              .filter((profile) => {
                const isRunning =
                  browserState.isClient && runningProfiles.has(profile.name);
                const isLaunching = launchingProfiles.has(profile.name);
                const isStopping = stoppingProfiles.has(profile.name);
                const isBrowserUpdating = isUpdating(profile.browser);
                return (
                  !isRunning &&
                  !isLaunching &&
                  !isStopping &&
                  !isBrowserUpdating
                );
              })
              .map((profile) => profile.name),
          )
        : new Set<string>();

      setSelectedProfiles(newSet);
      setShowCheckboxes(checked);

      // Notify parent component
      if (onSelectedProfilesChange) {
        onSelectedProfilesChange(Array.from(newSet));
      }
    },
    [
      filteredData,
      onSelectedProfilesChange,
      browserState.isClient,
      runningProfiles,
      launchingProfiles,
      stoppingProfiles,
      isUpdating,
    ],
  );

  // Memoize selectableProfiles calculation
  const selectableProfiles = React.useMemo(() => {
    return filteredData.filter((profile) => {
      const isRunning =
        browserState.isClient && runningProfiles.has(profile.name);
      const isLaunching = launchingProfiles.has(profile.name);
      const isStopping = stoppingProfiles.has(profile.name);
      const isBrowserUpdating = isUpdating(profile.browser);
      return !isRunning && !isLaunching && !isStopping && !isBrowserUpdating;
    });
  }, [
    filteredData,
    browserState.isClient,
    runningProfiles,
    launchingProfiles,
    stoppingProfiles,
    isUpdating,
  ]);

  // Stable handlers that don't change on every render
  const stableHandlers = React.useMemo(
    () => ({
      handleToggleAll,
      handleCheckboxChange,
      handleIconClick,
      handleProxySelection,
      handleRename,
      setStoppingProfiles,
      setLaunchingProfiles,
      setProfileToRename,
      setNewProfileName,
      setRenameError,
      setProfileToDelete,
      setOpenProxySelectorFor,
    }),
    [
      handleToggleAll,
      handleCheckboxChange,
      handleIconClick,
      handleProxySelection,
      handleRename,
    ],
  );

  const columns: ColumnDef<BrowserProfile>[] = React.useMemo(
    () => [
      {
        id: "select",
        header: () => {
          return (
            <span>
              <Checkbox
                checked={
                  selectedProfiles.size === selectableProfiles.length &&
                  selectableProfiles.length !== 0
                }
                onCheckedChange={(value) =>
                  stableHandlers.handleToggleAll(!!value)
                }
                aria-label="Select all"
                className="cursor-pointer"
              />
            </span>
          );
        },
        cell: ({ row, table }) => {
          const profile = row.original;
          const browser = profile.browser;
          const IconComponent = getBrowserIcon(browser);

          // Get dynamic state from table meta
          const tableMeta = table.options.meta as TableMeta;
          const isSelected =
            tableMeta?.selectedProfiles?.has(profile.name) || false;
          const isRunning =
            tableMeta?.browserState?.isClient &&
            tableMeta?.runningProfiles?.has(profile.name);
          const isLaunching =
            tableMeta?.launchingProfiles?.has(profile.name) || false;
          const isStopping =
            tableMeta?.stoppingProfiles?.has(profile.name) || false;
          const isBrowserUpdating = tableMeta?.isUpdating?.(browser) || false;
          const isDisabled =
            isRunning || isLaunching || isStopping || isBrowserUpdating;
          const showCheckboxes = tableMeta?.showCheckboxes || false;

          // Show tooltip for disabled profiles
          if (isDisabled) {
            const tooltipMessage = isRunning
              ? "Can't modify running profile"
              : isLaunching
                ? "Can't modify profile while launching"
                : isStopping
                  ? "Can't modify profile while stopping"
                  : "Can't modify profile while browser is updating";

            return (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="flex justify-center items-center w-4 h-4 cursor-not-allowed">
                    {IconComponent && (
                      <IconComponent className="w-4 h-4 opacity-50" />
                    )}
                  </span>
                </TooltipTrigger>
                <TooltipContent>
                  <p>{tooltipMessage}</p>
                </TooltipContent>
              </Tooltip>
            );
          }

          if (showCheckboxes || isSelected) {
            return (
              <span className="flex justify-center items-center w-4 h-4">
                <Checkbox
                  checked={isSelected}
                  onCheckedChange={(value) =>
                    stableHandlers.handleCheckboxChange(profile.name, !!value)
                  }
                  aria-label="Select row"
                  className="w-4 h-4"
                />
              </span>
            );
          }

          return (
            <span className="flex relative justify-center items-center w-4 h-4">
              <button
                type="button"
                className="flex justify-center items-center p-0 border-none cursor-pointer"
                onClick={() => stableHandlers.handleIconClick(profile.name)}
                aria-label="Select profile"
              >
                <span className="w-4 h-4 group">
                  {IconComponent && (
                    <IconComponent className="w-4 h-4 group-hover:hidden" />
                  )}
                  <span className="peer border-input dark:bg-input/30 dark:data-[state=checked]:bg-primary size-4 shrink-0 rounded-[4px] border shadow-xs transition-shadow outline-none w-4 h-4 hidden group-hover:block pointer-events-none items-center justify-center duration-200" />
                </span>
              </button>
            </span>
          );
        },
        enableSorting: false,
        enableHiding: false,
        size: 40,
      },
      {
        id: "actions",
        cell: ({ row, table }) => {
          const profile = row.original;
          const tableMeta = table.options.meta as TableMeta;
          const isRunning =
            tableMeta?.browserState?.isClient &&
            tableMeta?.runningProfiles?.has(profile.name);
          const isLaunching =
            tableMeta?.launchingProfiles?.has(profile.name) || false;
          const isStopping =
            tableMeta?.stoppingProfiles?.has(profile.name) || false;
          const canLaunch =
            tableMeta?.browserState?.canLaunchProfile(profile) || false;
          const tooltipContent =
            tableMeta?.browserState?.getLaunchTooltipContent(profile);

          const handleLaunchClick = async () => {
            if (isRunning) {
              console.log(
                `Stopping ${profile.browser} profile: ${profile.name}`,
              );
              stableHandlers.setStoppingProfiles((prev: Set<string>) =>
                new Set(prev).add(profile.name),
              );
              try {
                await tableMeta?.onKillProfile(profile);
                console.log(
                  `Successfully stopped ${profile.browser} profile: ${profile.name}`,
                );
              } catch (error) {
                console.error(
                  `Failed to stop ${profile.browser} profile: ${profile.name}`,
                  error,
                );
              } finally {
                stableHandlers.setStoppingProfiles((prev: Set<string>) => {
                  const next = new Set(prev);
                  next.delete(profile.name);
                  return next;
                });
              }
            } else {
              console.log(
                `Launching ${profile.browser} profile: ${profile.name}`,
              );
              stableHandlers.setLaunchingProfiles((prev: Set<string>) =>
                new Set(prev).add(profile.name),
              );
              try {
                await tableMeta?.onLaunchProfile(profile);
                console.log(
                  `Successfully launched ${profile.browser} profile: ${profile.name}`,
                );
              } catch (error) {
                console.error(
                  `Failed to launch ${profile.browser} profile: ${profile.name}`,
                  error,
                );
              } finally {
                stableHandlers.setLaunchingProfiles((prev: Set<string>) => {
                  const next = new Set(prev);
                  next.delete(profile.name);
                  return next;
                });
              }
            }
          };

          return (
            <div className="flex gap-2 items-center">
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="inline-flex">
                    <RippleButton
                      variant={isRunning ? "destructive" : "default"}
                      size="sm"
                      disabled={!canLaunch || isLaunching || isStopping}
                      className={cn(
                        "cursor-pointer min-w-[70px]",
                        !canLaunch && "opacity-50",
                      )}
                      onClick={() => void handleLaunchClick()}
                    >
                      {isLaunching || isStopping ? (
                        <div className="flex gap-1 items-center">
                          <div className="w-3 h-3 rounded-full border border-current animate-spin border-t-transparent" />
                        </div>
                      ) : isRunning ? (
                        "Stop"
                      ) : (
                        "Launch"
                      )}
                    </RippleButton>
                  </span>
                </TooltipTrigger>
                {tooltipContent && (
                  <TooltipContent>{tooltipContent}</TooltipContent>
                )}
              </Tooltip>
            </div>
          );
        },
      },
      {
        accessorKey: "name",
        header: ({ column }) => {
          return (
            <Button
              variant="ghost"
              onClick={() =>
                column.toggleSorting(column.getIsSorted() === "asc")
              }
              className="justify-start p-0 h-auto font-semibold text-left cursor-pointer"
            >
              Name
              {column.getIsSorted() === "asc" ? (
                <LuChevronUp className="ml-2 w-4 h-4" />
              ) : column.getIsSorted() === "desc" ? (
                <LuChevronDown className="ml-2 w-4 h-4" />
              ) : null}
            </Button>
          );
        },
        enableSorting: true,
        sortingFn: "alphanumeric",
        cell: ({ row, table }) => {
          const profile = row.original as BrowserProfile;
          const rawName: string = row.getValue("name");
          const name = getBrowserDisplayName(rawName);
          const tableMeta = table.options.meta as TableMeta;
          const isEditing = tableMeta?.profileToRename?.name === profile.name;
          const newProfileName = tableMeta?.newProfileName || "";
          const renameError = tableMeta?.renameError || null;
          const isRenamingSaving = tableMeta?.isRenamingSaving || false;

          if (isEditing) {
            const isSaveDisabled =
              isRenamingSaving ||
              newProfileName.trim().length === 0 ||
              newProfileName.trim() === profile.name;

            return (
              <div
                ref={renameContainerRef}
                className="overflow-visible relative"
              >
                <Input
                  autoFocus
                  value={newProfileName}
                  onChange={(e) => {
                    stableHandlers.setNewProfileName(e.target.value);
                    if (renameError) stableHandlers.setRenameError(null);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      void stableHandlers.handleRename();
                    } else if (e.key === "Escape") {
                      stableHandlers.setProfileToRename(null);
                      stableHandlers.setNewProfileName("");
                      stableHandlers.setRenameError(null);
                    }
                  }}
                  className="inline-block w-full"
                />
                <div className="flex absolute right-0 top-full z-50 gap-1 translate-y-[30%] opacity-100 bg-black rounded-md">
                  <LoadingButton
                    isLoading={isRenamingSaving}
                    size="sm"
                    variant="default"
                    disabled={isSaveDisabled}
                    className="cursor-pointer [&[disabled]]:bg-primary/80"
                    onClick={() => void stableHandlers.handleRename()}
                  >
                    Save
                  </LoadingButton>
                </div>
              </div>
            );
          }

          const display =
            name.length < 20 ? (
              <div className="font-medium text-left">{name}</div>
            ) : (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span>{trimName(name, 20)}</span>
                </TooltipTrigger>
                <TooltipContent>{name}</TooltipContent>
              </Tooltip>
            );

          const isRunning =
            tableMeta?.browserState?.isClient &&
            tableMeta?.runningProfiles?.has(profile.name);
          const isLaunching =
            tableMeta?.launchingProfiles?.has(profile.name) || false;
          const isStopping =
            tableMeta?.stoppingProfiles?.has(profile.name) || false;
          const isBrowserUpdating =
            tableMeta?.isUpdating?.(profile.browser) || false;
          const isDisabled =
            isRunning || isLaunching || isStopping || isBrowserUpdating;

          return (
            <button
              type="button"
              className={cn(
                "p-2 mr-auto w-full text-left bg-transparent rounded border-none",
                isDisabled
                  ? "opacity-60 cursor-not-allowed"
                  : "cursor-pointer hover:bg-accent/50",
              )}
              onClick={() => {
                if (isDisabled) return;
                stableHandlers.setProfileToRename(profile);
                stableHandlers.setNewProfileName(profile.name);
                stableHandlers.setRenameError(null);
              }}
              onKeyDown={(e) => {
                if (isDisabled) return;
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  stableHandlers.setProfileToRename(profile);
                  stableHandlers.setNewProfileName(profile.name);
                  stableHandlers.setRenameError(null);
                }
              }}
            >
              {display}
            </button>
          );
        },
      },
      {
        id: "tags",
        header: "Tags",
        cell: ({ row, table }) => {
          const profile = row.original;
          const tableMeta = table.options.meta as TableMeta;
          const isRunning =
            tableMeta?.browserState?.isClient &&
            tableMeta?.runningProfiles?.has(profile.name);
          const isLaunching =
            tableMeta?.launchingProfiles?.has(profile.name) || false;
          const isStopping =
            tableMeta?.stoppingProfiles?.has(profile.name) || false;
          const isBrowserUpdating =
            tableMeta?.isUpdating?.(profile.browser) || false;
          const isDisabled =
            isRunning || isLaunching || isStopping || isBrowserUpdating;

          return (
            <TagsCell
              profile={profile}
              isDisabled={isDisabled}
              tagsOverrides={tableMeta?.tagsOverrides || {}}
              allTags={tableMeta?.allTags || []}
              setAllTags={tableMeta?.setAllTags || (() => {})}
              openTagsEditorFor={tableMeta?.openTagsEditorFor || null}
              setOpenTagsEditorFor={
                tableMeta?.setOpenTagsEditorFor || (() => {})
              }
              setTagsOverrides={tableMeta?.setTagsOverrides || (() => {})}
            />
          );
        },
      },
      {
        accessorKey: "browser",
        header: ({ column }) => {
          return (
            <Button
              variant="ghost"
              onClick={() =>
                column.toggleSorting(column.getIsSorted() === "asc")
              }
              className="justify-start p-0 h-auto font-semibold text-left cursor-pointer"
            >
              Browser
              {column.getIsSorted() === "asc" ? (
                <LuChevronUp className="ml-2 w-4 h-4" />
              ) : column.getIsSorted() === "desc" ? (
                <LuChevronDown className="ml-2 w-4 h-4" />
              ) : null}
            </Button>
          );
        },
        cell: ({ row }) => {
          const browser: string = row.getValue("browser");
          const name = getBrowserDisplayName(browser);
          if (name.length < 20) {
            return (
              <div className="flex items-center">
                <span>{name}</span>
              </div>
            );
          }

          return (
            <Tooltip>
              <TooltipTrigger asChild>
                <span>{trimName(name, 20)}</span>
              </TooltipTrigger>
              <TooltipContent>{name}</TooltipContent>
            </Tooltip>
          );
        },
        enableSorting: true,
        sortingFn: (rowA, rowB, columnId) => {
          const browserA: string = rowA.getValue(columnId);
          const browserB: string = rowB.getValue(columnId);
          return getBrowserDisplayName(browserA).localeCompare(
            getBrowserDisplayName(browserB),
          );
        },
      },
      {
        id: "proxy",
        header: "Proxy",
        cell: ({ row, table }) => {
          const profile = row.original;
          const tableMeta = table.options.meta as TableMeta;
          const isRunning =
            tableMeta?.browserState?.isClient &&
            tableMeta?.runningProfiles?.has(profile.name);
          const isLaunching =
            tableMeta?.launchingProfiles?.has(profile.name) || false;
          const isStopping =
            tableMeta?.stoppingProfiles?.has(profile.name) || false;
          const isBrowserUpdating =
            tableMeta?.isUpdating?.(profile.browser) || false;
          const isDisabled =
            isRunning || isLaunching || isStopping || isBrowserUpdating;

          const proxyOverrides = tableMeta?.proxyOverrides || {};
          const storedProxies = tableMeta?.storedProxies || [];
          const openProxySelectorFor = tableMeta?.openProxySelectorFor || null;

          const hasOverride = Object.hasOwn(proxyOverrides, profile.name);
          const effectiveProxyId = hasOverride
            ? proxyOverrides[profile.name]
            : (profile.proxy_id ?? null);
          const effectiveProxy = effectiveProxyId
            ? (storedProxies.find((p) => p.id === effectiveProxyId) ?? null)
            : null;
          const displayName =
            profile.browser === "tor-browser"
              ? "Not supported"
              : effectiveProxy
                ? effectiveProxy.name
                : "Not Selected";
          const profileHasProxy = Boolean(effectiveProxy);
          const tooltipText =
            profile.browser === "tor-browser"
              ? "Proxies are not supported for TOR browser"
              : profileHasProxy && effectiveProxy
                ? `${effectiveProxy.name} (${effectiveProxy.proxy_settings.proxy_type.toUpperCase()})`
                : "";
          const isSelectorOpen = openProxySelectorFor === profile.name;

          if (profile.browser === "tor-browser") {
            return (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="flex gap-2 items-center">
                    <span className="text-sm text-muted-foreground">
                      Not supported
                    </span>
                  </span>
                </TooltipTrigger>
                {tooltipText && <TooltipContent>{tooltipText}</TooltipContent>}
              </Tooltip>
            );
          }

          return (
            <Popover
              open={isSelectorOpen}
              onOpenChange={(open) =>
                stableHandlers.setOpenProxySelectorFor(
                  open ? profile.name : null,
                )
              }
            >
              <Tooltip>
                <TooltipTrigger asChild>
                  <PopoverTrigger asChild>
                    <span
                      className={cn(
                        "flex gap-2 items-center p-2 rounded",
                        isDisabled
                          ? "opacity-60 cursor-not-allowed pointer-events-none"
                          : "cursor-pointer hover:bg-accent/50",
                      )}
                    >
                      {profileHasProxy && (
                        <CiCircleCheck className="w-4 h-4 text-green-500" />
                      )}
                      {displayName.length > 18 ? (
                        <span className="text-sm truncate text-muted-foreground max-w-[140px]">
                          {displayName.slice(0, 18)}...
                        </span>
                      ) : (
                        <span className="text-sm text-muted-foreground">
                          {displayName}
                        </span>
                      )}
                    </span>
                  </PopoverTrigger>
                </TooltipTrigger>
                {tooltipText && <TooltipContent>{tooltipText}</TooltipContent>}
              </Tooltip>

              {!isDisabled && (
                <PopoverContent className="w-[240px] p-0" align="start">
                  <Command>
                    <CommandInput placeholder="Search proxies..." />
                    <CommandList>
                      <CommandEmpty>No proxies found.</CommandEmpty>
                      <CommandGroup>
                        <CommandItem
                          value="__none__"
                          onSelect={() =>
                            void stableHandlers.handleProxySelection(
                              profile.name,
                              null,
                            )
                          }
                        >
                          <LuCheck
                            className={cn(
                              "mr-2 h-4 w-4",
                              effectiveProxyId === null
                                ? "opacity-100"
                                : "opacity-0",
                            )}
                          />
                          No Proxy
                        </CommandItem>
                        {storedProxies.map((proxy) => (
                          <CommandItem
                            key={proxy.id}
                            value={proxy.name}
                            onSelect={() =>
                              void stableHandlers.handleProxySelection(
                                profile.name,
                                proxy.id,
                              )
                            }
                          >
                            <LuCheck
                              className={cn(
                                "mr-2 h-4 w-4",
                                effectiveProxyId === proxy.id
                                  ? "opacity-100"
                                  : "opacity-0",
                              )}
                            />
                            {proxy.name}
                          </CommandItem>
                        ))}
                      </CommandGroup>
                    </CommandList>
                  </Command>
                </PopoverContent>
              )}
            </Popover>
          );
        },
      },
      {
        id: "settings",
        cell: ({ row, table }) => {
          const profile = row.original;
          const tableMeta = table.options.meta as TableMeta;
          const isRunning =
            tableMeta?.browserState?.isClient &&
            tableMeta?.runningProfiles?.has(profile.name);
          const isBrowserUpdating =
            tableMeta?.browserState?.isClient &&
            tableMeta?.isUpdating?.(profile.browser);
          const isLaunching =
            tableMeta?.launchingProfiles?.has(profile.name) || false;
          const isStopping =
            tableMeta?.stoppingProfiles?.has(profile.name) || false;
          const isDisabled =
            isRunning || isLaunching || isStopping || isBrowserUpdating;

          return (
            <div className="flex justify-end items-center">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    variant="ghost"
                    className="p-0 w-8 h-8"
                    disabled={!tableMeta?.browserState?.isClient}
                  >
                    <span className="sr-only">Open menu</span>
                    <IoEllipsisHorizontal className="w-4 h-4" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem
                    onClick={() => {
                      if (tableMeta?.onAssignProfilesToGroup) {
                        tableMeta.onAssignProfilesToGroup([profile.name]);
                      }
                    }}
                    disabled={isDisabled}
                  >
                    Assign to Group
                  </DropdownMenuItem>
                  {profile.browser === "camoufox" &&
                    tableMeta?.onConfigureCamoufox && (
                      <DropdownMenuItem
                        onClick={() => {
                          tableMeta.onConfigureCamoufox?.(profile);
                        }}
                        disabled={isDisabled}
                      >
                        Configure Fingerprint
                      </DropdownMenuItem>
                    )}
                  <DropdownMenuItem
                    onClick={() => {
                      stableHandlers.setProfileToDelete(profile);
                    }}
                    disabled={isDisabled}
                  >
                    Delete
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          );
        },
      },
    ],
    [stableHandlers, selectableProfiles, selectedProfiles],
  );

  // Memoize table meta to prevent unnecessary re-renders
  const tableMeta = React.useMemo(
    () => ({
      tagsOverrides,
      allTags,
      setAllTags,
      openTagsEditorFor,
      setOpenTagsEditorFor,
      setTagsOverrides,
      // Include all the state needed by columns
      selectedProfiles,
      showCheckboxes,
      browserState,
      runningProfiles,
      launchingProfiles,
      stoppingProfiles,
      isUpdating,
      proxyOverrides,
      storedProxies,
      openProxySelectorFor,
      profileToRename,
      newProfileName,
      renameError,
      isRenamingSaving,
      onLaunchProfile,
      onKillProfile,
      onConfigureCamoufox,
      onAssignProfilesToGroup,
    }),
    [
      tagsOverrides,
      allTags,
      openTagsEditorFor,
      selectedProfiles,
      showCheckboxes,
      browserState,
      runningProfiles,
      launchingProfiles,
      stoppingProfiles,
      isUpdating,
      proxyOverrides,
      storedProxies,
      openProxySelectorFor,
      profileToRename,
      newProfileName,
      renameError,
      isRenamingSaving,
      onLaunchProfile,
      onKillProfile,
      onConfigureCamoufox,
      onAssignProfilesToGroup,
    ],
  );

  const table = useReactTable({
    data: filteredData,
    columns,
    state: {
      sorting,
    },
    meta: tableMeta,
    onSortingChange: handleSortingChange,
    getSortedRowModel: getSortedRowModel(),
    getCoreRowModel: getCoreRowModel(),
  });

  const platform = getCurrentOS();

  return (
    <>
      <ScrollArea
        className={cn(
          "rounded-md border [&>div[data-slot='scroll-area-viewport']>div]:overflow-visible",
          platform === "macos" ? "h-[340px]" : "h-[280px]",
        )}
      >
        <Table className="overflow-visible">
          <TableHeader className="overflow-visible">
            {table.getHeaderGroups().map((headerGroup) => (
              <TableRow key={headerGroup.id} className="overflow-visible">
                {headerGroup.headers.map((header) => {
                  return (
                    <TableHead key={header.id}>
                      {header.isPlaceholder
                        ? null
                        : flexRender(
                            header.column.columnDef.header,
                            header.getContext(),
                          )}
                    </TableHead>
                  );
                })}
              </TableRow>
            ))}
          </TableHeader>
          <TableBody className="overflow-visible">
            {table.getRowModel().rows?.length ? (
              table.getRowModel().rows.map((row) => (
                <TableRow
                  key={row.id}
                  data-state={row.getIsSelected() && "selected"}
                  className="overflow-visible hover:bg-accent/50"
                >
                  {row.getVisibleCells().map((cell) => (
                    <TableCell key={cell.id} className="overflow-visible">
                      {flexRender(
                        cell.column.columnDef.cell,
                        cell.getContext(),
                      )}
                    </TableCell>
                  ))}
                </TableRow>
              ))
            ) : (
              <TableRow>
                <TableCell
                  colSpan={columns.length}
                  className="h-24 text-center"
                >
                  No profiles found.
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </ScrollArea>
      <DeleteConfirmationDialog
        isOpen={profileToDelete !== null}
        onClose={() => setProfileToDelete(null)}
        onConfirm={handleDelete}
        title="Delete Profile"
        description={`This action cannot be undone. This will permanently delete the profile "${profileToDelete?.name}" and all its associated data.`}
        confirmButtonText="Delete Profile"
        isLoading={isDeleting}
      />
    </>
  );
}
