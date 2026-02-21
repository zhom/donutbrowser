"use client";

import {
  type ColumnDef,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  type RowSelectionState,
  type SortingState,
  useReactTable,
} from "@tanstack/react-table";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import type { Dispatch, SetStateAction } from "react";
import * as React from "react";
import { FaApple, FaLinux, FaWindows } from "react-icons/fa";
import { FiWifi } from "react-icons/fi";
import { IoEllipsisHorizontal } from "react-icons/io5";
import {
  LuCheck,
  LuChevronDown,
  LuChevronUp,
  LuCookie,
  LuTrash2,
  LuUsers,
} from "react-icons/lu";
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
import { ProBadge } from "@/components/ui/pro-badge";
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
import { useProxyEvents } from "@/hooks/use-proxy-events";
import { useTableSorting } from "@/hooks/use-table-sorting";
import { useVpnEvents } from "@/hooks/use-vpn-events";
import {
  getBrowserDisplayName,
  getBrowserIcon,
  getCurrentOS,
  getOSDisplayName,
  isCrossOsProfile,
} from "@/lib/browser-utils";
import { formatRelativeTime } from "@/lib/flag-utils";
import { trimName } from "@/lib/name-utils";
import { cn } from "@/lib/utils";
import type {
  BrowserProfile,
  LocationItem,
  ProxyCheckResult,
  StoredProxy,
  TrafficSnapshot,
  VpnConfig,
} from "@/types";
import { BandwidthMiniChart } from "./bandwidth-mini-chart";
import {
  DataTableActionBar,
  DataTableActionBarAction,
  DataTableActionBarSelection,
} from "./data-table-action-bar";
import MultipleSelector, { type Option } from "./multiple-selector";
import { ProxyCheckButton } from "./proxy-check-button";
import { TrafficDetailsDialog } from "./traffic-details-dialog";
import { Input } from "./ui/input";
import { RippleButton } from "./ui/ripple";

// Stable table meta type to pass volatile state/handlers into TanStack Table without
// causing column definitions to be recreated on every render.
type TableMeta = {
  selectedProfiles: string[];
  selectableCount: number;
  showCheckboxes: boolean;
  isClient: boolean;
  runningProfiles: Set<string>;
  launchingProfiles: Set<string>;
  stoppingProfiles: Set<string>;
  isUpdating: (browser: string) => boolean;
  browserState: ReturnType<typeof useBrowserState>;

  // Tags editor state
  tagsOverrides: Record<string, string[]>;
  allTags: string[];
  openTagsEditorFor: string | null;
  setAllTags: React.Dispatch<React.SetStateAction<string[]>>;
  setOpenTagsEditorFor: React.Dispatch<React.SetStateAction<string | null>>;
  setTagsOverrides: React.Dispatch<
    React.SetStateAction<Record<string, string[]>>
  >;

  // Note editor state
  noteOverrides: Record<string, string | null>;
  openNoteEditorFor: string | null;
  setOpenNoteEditorFor: React.Dispatch<React.SetStateAction<string | null>>;
  setNoteOverrides: React.Dispatch<
    React.SetStateAction<Record<string, string | null>>
  >;

  // Proxy selector state
  openProxySelectorFor: string | null;
  setOpenProxySelectorFor: React.Dispatch<React.SetStateAction<string | null>>;
  proxyOverrides: Record<string, string | null>;
  storedProxies: StoredProxy[];
  handleProxySelection: (
    profileId: string,
    proxyId: string | null,
  ) => void | Promise<void>;
  checkingProfileId: string | null;
  proxyCheckResults: Record<string, ProxyCheckResult>;

  // VPN selector state
  vpnConfigs: VpnConfig[];
  vpnOverrides: Record<string, string | null>;
  handleVpnSelection: (
    profileId: string,
    vpnId: string | null,
  ) => void | Promise<void>;

  // Selection helpers
  isProfileSelected: (id: string) => boolean;
  handleToggleAll: (checked: boolean) => void;
  handleCheckboxChange: (id: string, checked: boolean) => void;
  handleIconClick: (id: string) => void;

  // Rename helpers
  handleRename: () => void | Promise<void>;
  setProfileToRename: React.Dispatch<
    React.SetStateAction<BrowserProfile | null>
  >;
  setNewProfileName: React.Dispatch<React.SetStateAction<string>>;
  setRenameError: React.Dispatch<React.SetStateAction<string | null>>;
  profileToRename: BrowserProfile | null;
  newProfileName: string;
  isRenamingSaving: boolean;
  renameError: string | null;

  // Launch/stop helpers
  setLaunchingProfiles: React.Dispatch<React.SetStateAction<Set<string>>>;
  setStoppingProfiles: React.Dispatch<React.SetStateAction<Set<string>>>;
  onKillProfile: (profile: BrowserProfile) => void | Promise<void>;
  onLaunchProfile: (profile: BrowserProfile) => void | Promise<void>;

  // Overflow actions
  onAssignProfilesToGroup?: (profileIds: string[]) => void;
  onConfigureCamoufox?: (profile: BrowserProfile) => void;
  onCloneProfile?: (profile: BrowserProfile) => void;
  onCopyCookiesToProfile?: (profile: BrowserProfile) => void;
  onImportCookies?: (profile: BrowserProfile) => void;
  onExportCookies?: (profile: BrowserProfile) => void;

  // Traffic snapshots (lightweight real-time data)
  trafficSnapshots: Record<string, TrafficSnapshot>;
  onOpenTrafficDialog?: (profileId: string) => void;

  // Sync
  syncStatuses: Record<string, { status: string; error?: string }>;
  onOpenProfileSyncDialog?: (profile: BrowserProfile) => void;
  onToggleProfileSync?: (profile: BrowserProfile) => void;
  crossOsUnlocked?: boolean;
  syncUnlocked?: boolean;

  // Country proxy creation (inline in proxy dropdown)
  countries: LocationItem[];
  canCreateLocationProxy: boolean;
  loadCountries: () => Promise<void>;
  handleCreateCountryProxy: (
    profileId: string,
    country: LocationItem,
  ) => Promise<void>;
};

type SyncStatusDot = { color: string; tooltip: string; animate: boolean };

function getProfileSyncStatusDot(
  profile: BrowserProfile,
  liveStatus:
    | "syncing"
    | "waiting"
    | "synced"
    | "error"
    | "disabled"
    | undefined,
  errorMessage?: string,
): SyncStatusDot | null {
  const status = liveStatus ?? (profile.sync_enabled ? "synced" : "disabled");

  switch (status) {
    case "syncing":
      return { color: "bg-yellow-500", tooltip: "Syncing...", animate: true };
    case "waiting":
      return {
        color: "bg-yellow-500",
        tooltip: "Waiting to sync",
        animate: false,
      };
    case "synced":
      return {
        color: "bg-green-500",
        tooltip: profile.last_sync
          ? `Synced ${new Date(profile.last_sync * 1000).toLocaleString()}`
          : "Synced",
        animate: false,
      };
    case "error":
      return {
        color: "bg-red-500",
        tooltip: errorMessage ? `Sync error: ${errorMessage}` : "Sync error",
        animate: false,
      };
    case "disabled":
      if (profile.last_sync) {
        return {
          color: "bg-gray-400",
          tooltip: `Sync disabled, last sync ${formatRelativeTime(profile.last_sync)}`,
          animate: false,
        };
      }
      return null;
    default:
      return null;
  }
}

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
    const effectiveTags: string[] = Object.hasOwn(tagsOverrides, profile.id)
      ? tagsOverrides[profile.id]
      : (profile.tags ?? []);

    const valueOptions: Option[] = React.useMemo(
      () => effectiveTags.map((t) => ({ value: t, label: t })),
      [effectiveTags],
    );
    const allOptions: Option[] = React.useMemo(
      () => allTags.map((t) => ({ value: t, label: t })),
      [allTags],
    );

    const onTagsChange = React.useCallback(
      async (newTagsRaw: string[]) => {
        // Dedupe tags
        const seen = new Set<string>();
        const newTags: string[] = [];
        for (const t of newTagsRaw) {
          if (!seen.has(t)) {
            seen.add(t);
            newTags.push(t);
          }
        }
        setTagsOverrides((prev) => ({ ...prev, [profile.id]: newTags }));
        try {
          await invoke<BrowserProfile>("update_profile_tags", {
            profileId: profile.id,
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
      [profile.id, setTagsOverrides, setAllTags],
    );

    const handleChange = React.useCallback(
      async (opts: Option[]) => {
        const newTagsRaw = opts.map((o) => o.value);
        await onTagsChange(newTagsRaw);
      },
      [onTagsChange],
    );

    const containerRef = React.useRef<HTMLDivElement | null>(null);
    const editorRef = React.useRef<HTMLDivElement | null>(null);
    const [visibleCount, setVisibleCount] = React.useState<number>(
      effectiveTags.length,
    );
    const [isFocused, setIsFocused] = React.useState(false);

    React.useLayoutEffect(() => {
      // Only measure when not editing this profile's tags
      if (openTagsEditorFor === profile.id) return;
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
    }, [effectiveTags, openTagsEditorFor, profile.id]);

    React.useEffect(() => {
      if (openTagsEditorFor !== profile.id) return;
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
    }, [openTagsEditorFor, profile.id, setOpenTagsEditorFor]);

    React.useEffect(() => {
      if (openTagsEditorFor === profile.id && editorRef.current) {
        // Focus the inner input of MultipleSelector on open
        const inputEl = editorRef.current.querySelector("input");
        if (inputEl) {
          (inputEl as HTMLInputElement).focus();
        }
      }
    }, [openTagsEditorFor, profile.id]);

    if (openTagsEditorFor !== profile.id) {
      const hiddenCount = Math.max(0, effectiveTags.length - visibleCount);
      const ButtonContent = (
        <button
          type="button"
          ref={containerRef as unknown as React.RefObject<HTMLButtonElement>}
          className={cn(
            "flex overflow-hidden gap-1 items-center px-2 py-1 h-6 w-full bg-transparent rounded border-none cursor-pointer",
            isDisabled
              ? "opacity-60 cursor-not-allowed"
              : "cursor-pointer hover:bg-accent/50",
          )}
          onClick={() => {
            if (!isDisabled) setOpenTagsEditorFor(profile.id);
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
      );

      return (
        <div className="w-40 h-6 cursor-pointer">
          <Tooltip>
            <TooltipTrigger asChild>{ButtonContent}</TooltipTrigger>
            {hiddenCount > 0 && (
              <TooltipContent className="max-w-[320px]">
                <div className="flex flex-wrap gap-1">
                  {effectiveTags.map((t) => (
                    <Badge
                      key={t}
                      variant="secondary"
                      className="px-2 py-0 text-xs"
                    >
                      {t}
                    </Badge>
                  ))}
                </div>
              </TooltipContent>
            )}
          </Tooltip>
        </div>
      );
    }

    return (
      <div
        className={cn(
          "w-40 h-6 relative",
          isDisabled && "opacity-60 pointer-events-none",
        )}
      >
        <div
          ref={editorRef}
          className="absolute top-0 left-0 z-50 w-40 min-h-6 bg-popover rounded-md shadow-md"
        >
          <MultipleSelector
            value={valueOptions}
            options={allOptions}
            onChange={(opts) => void handleChange(opts)}
            creatable
            selectFirstItem={false}
            placeholder={effectiveTags.length === 0 ? "Add tags" : ""}
            className={cn(
              "bg-transparent border-0! focus-within:ring-0!",
              "[&_div:first-child]:border-0! [&_div:first-child]:ring-0! [&_div:first-child]:focus-within:ring-0!",
              "[&_div:first-child]:min-h-6! [&_div:first-child]:px-2! [&_div:first-child]:py-1!",
              "[&_div:first-child>div]:items-center [&_div:first-child>div]:h-6!",
              "[&_input]:ml-0! [&_input]:mt-0! [&_input]:px-0!",
              !isFocused && "[&_div:first-child>div]:justify-center",
            )}
            badgeClassName="shrink-0"
            inputProps={{
              className: "!py-0 text-sm caret-current !ml-0 !mt-0 !px-0",
              onKeyDown: (e) => {
                if (e.key === "Escape") setOpenTagsEditorFor(null);
              },
              onFocus: () => setIsFocused(true),
              onBlur: () => setIsFocused(false),
            }}
          />
        </div>
      </div>
    );
  },
);

TagsCell.displayName = "TagsCell";

const NonHoverableTooltip = React.memo<{
  children: React.ReactNode;
  content: React.ReactNode;
  sideOffset?: number;
  alignOffset?: number;
  horizontalOffset?: number;
}>(
  ({
    children,
    content,
    sideOffset = 4,
    alignOffset = 0,
    horizontalOffset = 0,
  }) => {
    const [isOpen, setIsOpen] = React.useState(false);

    return (
      <Tooltip open={isOpen} onOpenChange={setIsOpen}>
        <TooltipTrigger
          asChild
          onMouseEnter={() => setIsOpen(true)}
          onMouseLeave={() => setIsOpen(false)}
        >
          {children}
        </TooltipTrigger>
        <TooltipContent
          sideOffset={sideOffset}
          alignOffset={alignOffset}
          arrowOffset={horizontalOffset}
          onPointerEnter={(e) => e.preventDefault()}
          onPointerLeave={() => setIsOpen(false)}
          className="pointer-events-none"
          style={
            horizontalOffset !== 0
              ? { transform: `translateX(${horizontalOffset}px)` }
              : undefined
          }
        >
          {content}
        </TooltipContent>
      </Tooltip>
    );
  },
);

NonHoverableTooltip.displayName = "NonHoverableTooltip";

const NoteCell = React.memo<{
  profile: BrowserProfile;
  isDisabled: boolean;
  noteOverrides: Record<string, string | null>;
  openNoteEditorFor: string | null;
  setOpenNoteEditorFor: React.Dispatch<React.SetStateAction<string | null>>;
  setNoteOverrides: React.Dispatch<
    React.SetStateAction<Record<string, string | null>>
  >;
}>(
  ({
    profile,
    isDisabled,
    noteOverrides,
    openNoteEditorFor,
    setOpenNoteEditorFor,
    setNoteOverrides,
  }) => {
    const effectiveNote: string | null = Object.hasOwn(
      noteOverrides,
      profile.id,
    )
      ? noteOverrides[profile.id]
      : (profile.note ?? null);

    const onNoteChange = React.useCallback(
      async (newNote: string | null) => {
        const trimmedNote = newNote?.trim() || null;
        setNoteOverrides((prev) => ({ ...prev, [profile.id]: trimmedNote }));
        try {
          await invoke<BrowserProfile>("update_profile_note", {
            profileId: profile.id,
            note: trimmedNote,
          });
        } catch (error) {
          console.error("Failed to update note:", error);
        }
      },
      [profile.id, setNoteOverrides],
    );

    const editorRef = React.useRef<HTMLDivElement | null>(null);
    const textareaRef = React.useRef<HTMLTextAreaElement | null>(null);
    const [noteValue, setNoteValue] = React.useState(effectiveNote || "");

    // Update local state when effective note changes (from outside)
    React.useEffect(() => {
      if (openNoteEditorFor !== profile.id) {
        setNoteValue(effectiveNote || "");
      }
    }, [effectiveNote, openNoteEditorFor, profile.id]);

    // Auto-resize textarea on open
    React.useEffect(() => {
      if (openNoteEditorFor === profile.id && textareaRef.current) {
        const textarea = textareaRef.current;
        textarea.style.height = "auto";
        textarea.style.height = `${Math.min(textarea.scrollHeight, 200)}px`;
      }
    }, [openNoteEditorFor, profile.id]);

    const handleTextareaChange = React.useCallback(
      (e: React.ChangeEvent<HTMLTextAreaElement>) => {
        const newValue = e.target.value;
        setNoteValue(newValue);
        // Auto-resize
        const textarea = e.target;
        textarea.style.height = "auto";
        textarea.style.height = `${Math.min(textarea.scrollHeight, 200)}px`;
      },
      [],
    );

    React.useEffect(() => {
      if (openNoteEditorFor !== profile.id) return;
      const handleClick = (e: MouseEvent) => {
        const target = e.target as Node | null;
        if (
          editorRef.current &&
          target &&
          !editorRef.current.contains(target)
        ) {
          const currentValue = textareaRef.current?.value || "";
          void onNoteChange(currentValue);
          setOpenNoteEditorFor(null);
        }
      };
      document.addEventListener("mousedown", handleClick);
      return () => document.removeEventListener("mousedown", handleClick);
    }, [openNoteEditorFor, profile.id, setOpenNoteEditorFor, onNoteChange]);

    React.useEffect(() => {
      if (openNoteEditorFor === profile.id && textareaRef.current) {
        textareaRef.current.focus();
        // Move cursor to end
        const len = textareaRef.current.value.length;
        textareaRef.current.setSelectionRange(len, len);
      }
    }, [openNoteEditorFor, profile.id]);

    const displayNote = effectiveNote || "";
    const trimmedNote =
      displayNote.length > 12 ? `${displayNote.slice(0, 12)}...` : displayNote;
    const showTooltip = displayNote.length > 12 || displayNote.length > 0;

    if (openNoteEditorFor !== profile.id) {
      return (
        <div className="w-24 min-h-6">
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                className={cn(
                  "flex items-start px-2 py-1 min-h-6 w-full bg-transparent rounded border-none text-left",
                  isDisabled
                    ? "opacity-60 cursor-not-allowed"
                    : "cursor-pointer hover:bg-accent/50",
                )}
                onClick={() => {
                  if (!isDisabled) {
                    setNoteValue(effectiveNote || "");
                    setOpenNoteEditorFor(profile.id);
                  }
                }}
              >
                <span
                  className={cn(
                    "text-sm wrap-break-word",
                    !effectiveNote && "text-muted-foreground",
                  )}
                >
                  {effectiveNote ? trimmedNote : "No Note"}
                </span>
              </button>
            </TooltipTrigger>
            {showTooltip && (
              <TooltipContent className="max-w-[320px]">
                <p className="whitespace-pre-wrap wrap-break-word">
                  {effectiveNote || "No Note"}
                </p>
              </TooltipContent>
            )}
          </Tooltip>
        </div>
      );
    }

    return (
      <div
        className={cn(
          "w-24 relative",
          isDisabled && "opacity-60 pointer-events-none",
        )}
      >
        <div
          ref={editorRef}
          className="absolute -top-[15px] -left-px z-50 w-60 min-h-6 bg-popover rounded-md shadow-md border"
        >
          <textarea
            ref={textareaRef}
            value={noteValue}
            onChange={handleTextareaChange}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                setNoteValue(effectiveNote || "");
                setOpenNoteEditorFor(null);
              } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                void onNoteChange(noteValue);
                setOpenNoteEditorFor(null);
              }
            }}
            onBlur={() => {
              void onNoteChange(noteValue);
              setOpenNoteEditorFor(null);
            }}
            placeholder="Add a note..."
            className="w-full min-h-6 max-h-[200px] px-2 py-1 text-sm bg-transparent border-0 resize-none focus:outline-none focus:ring-0"
            style={{
              overflow: "auto",
            }}
            rows={1}
          />
        </div>
      </div>
    );
  },
);

NoteCell.displayName = "NoteCell";

interface ProfilesDataTableProps {
  profiles: BrowserProfile[];
  onLaunchProfile: (profile: BrowserProfile) => void | Promise<void>;
  onKillProfile: (profile: BrowserProfile) => void | Promise<void>;
  onCloneProfile: (profile: BrowserProfile) => void | Promise<void>;
  onDeleteProfile: (profile: BrowserProfile) => void | Promise<void>;
  onRenameProfile: (profileId: string, newName: string) => Promise<void>;
  onConfigureCamoufox: (profile: BrowserProfile) => void;
  onCopyCookiesToProfile?: (profile: BrowserProfile) => void;
  onImportCookies?: (profile: BrowserProfile) => void;
  onExportCookies?: (profile: BrowserProfile) => void;
  runningProfiles: Set<string>;
  isUpdating: (browser: string) => boolean;
  onDeleteSelectedProfiles: (profileIds: string[]) => Promise<void>;
  onAssignProfilesToGroup: (profileIds: string[]) => void;
  selectedGroupId: string | null;
  selectedProfiles: string[];
  onSelectedProfilesChange: Dispatch<SetStateAction<string[]>>;
  onBulkDelete?: () => void;
  onBulkGroupAssignment?: () => void;
  onBulkProxyAssignment?: () => void;
  onBulkCopyCookies?: () => void;
  onOpenProfileSyncDialog?: (profile: BrowserProfile) => void;
  onToggleProfileSync?: (profile: BrowserProfile) => void;
  crossOsUnlocked?: boolean;
  syncUnlocked?: boolean;
}

export function ProfilesDataTable({
  profiles,
  onLaunchProfile,
  onKillProfile,
  onCloneProfile,
  onDeleteProfile,
  onRenameProfile,
  onConfigureCamoufox,
  onCopyCookiesToProfile,
  onImportCookies,
  onExportCookies,
  runningProfiles,
  isUpdating,
  onAssignProfilesToGroup,
  selectedProfiles,
  onSelectedProfilesChange,
  onBulkDelete,
  onBulkGroupAssignment,
  onBulkProxyAssignment,
  onBulkCopyCookies,
  onOpenProfileSyncDialog,
  onToggleProfileSync,
  crossOsUnlocked = false,
  syncUnlocked = false,
}: ProfilesDataTableProps) {
  const { getTableSorting, updateSorting, isLoaded } = useTableSorting();
  const [sorting, setSorting] = React.useState<SortingState>([]);

  // Sync external selectedProfiles with table's row selection state
  const [rowSelection, setRowSelection] = React.useState<RowSelectionState>({});
  const prevSelectedProfilesRef = React.useRef<string[]>(selectedProfiles);

  // Update row selection when external selectedProfiles changes
  React.useEffect(() => {
    // Only update if selectedProfiles actually changed
    if (
      prevSelectedProfilesRef.current.length !== selectedProfiles.length ||
      !prevSelectedProfilesRef.current.every((id) =>
        selectedProfiles.includes(id),
      )
    ) {
      const newSelection: RowSelectionState = {};
      for (const profileId of selectedProfiles) {
        newSelection[profileId] = true;
      }
      setRowSelection(newSelection);
      prevSelectedProfilesRef.current = selectedProfiles;
    }
  }, [selectedProfiles]);

  // Update external selectedProfiles when table selection changes
  const handleRowSelectionChange = React.useCallback(
    (updater: React.SetStateAction<RowSelectionState>) => {
      setRowSelection((prevSelection) => {
        const newSelection =
          typeof updater === "function" ? updater(prevSelection) : updater;

        const selectedIds = Object.keys(newSelection).filter(
          (id) => newSelection[id],
        );

        // Only update external state if selection actually changed
        const prevIds = Object.keys(prevSelection).filter(
          (id) => prevSelection[id],
        );

        if (
          selectedIds.length !== prevIds.length ||
          !selectedIds.every((id) => prevIds.includes(id))
        ) {
          onSelectedProfilesChange(selectedIds);
        }

        return newSelection;
      });
    },
    [onSelectedProfilesChange],
  );
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

  const { storedProxies } = useProxyEvents();
  const { vpnConfigs } = useVpnEvents();

  const [proxyOverrides, setProxyOverrides] = React.useState<
    Record<string, string | null>
  >({});
  const [vpnOverrides, setVpnOverrides] = React.useState<
    Record<string, string | null>
  >({});
  const [showCheckboxes, setShowCheckboxes] = React.useState(false);
  const [tagsOverrides, setTagsOverrides] = React.useState<
    Record<string, string[]>
  >({});
  const [allTags, setAllTags] = React.useState<string[]>([]);
  const [openTagsEditorFor, setOpenTagsEditorFor] = React.useState<
    string | null
  >(null);
  const [openProxySelectorFor, setOpenProxySelectorFor] = React.useState<
    string | null
  >(null);
  const [checkingProfileId, setCheckingProfileId] = React.useState<
    string | null
  >(null);
  const [proxyCheckResults, setProxyCheckResults] = React.useState<
    Record<string, ProxyCheckResult>
  >({});
  const [noteOverrides, setNoteOverrides] = React.useState<
    Record<string, string | null>
  >({});
  const [openNoteEditorFor, setOpenNoteEditorFor] = React.useState<
    string | null
  >(null);
  const [trafficSnapshots, setTrafficSnapshots] = React.useState<
    Record<string, TrafficSnapshot>
  >({});
  const [trafficDialogProfile, setTrafficDialogProfile] = React.useState<{
    id: string;
    name?: string;
  } | null>(null);
  const [syncStatuses, setSyncStatuses] = React.useState<
    Record<string, { status: string; error?: string }>
  >({});

  // Country proxy creation state (for inline proxy creation in dropdown)
  const [countries, setCountries] = React.useState<LocationItem[]>([]);
  const [countriesLoaded, setCountriesLoaded] = React.useState(false);
  const hasCloudProxy = storedProxies.some((p) => p.is_cloud_managed);
  const canCreateLocationProxy = hasCloudProxy || crossOsUnlocked;

  const loadCountries = React.useCallback(async () => {
    if (countriesLoaded || !canCreateLocationProxy) return;
    try {
      const data = await invoke<LocationItem[]>("cloud_get_countries");
      setCountries(data);
      setCountriesLoaded(true);
    } catch (e) {
      console.error("Failed to load countries:", e);
    }
  }, [countriesLoaded, canCreateLocationProxy]);

  // Load cached check results for proxies
  React.useEffect(() => {
    const loadCachedResults = async () => {
      const results: Record<string, ProxyCheckResult> = {};
      const proxyIds = new Set<string>();
      for (const profile of profiles) {
        if (profile.proxy_id) {
          proxyIds.add(profile.proxy_id);
        }
      }
      for (const proxyId of proxyIds) {
        try {
          const cached = await invoke<ProxyCheckResult | null>(
            "get_cached_proxy_check",
            { proxyId },
          );
          if (cached) {
            results[proxyId] = cached;
          }
        } catch (_error) {
          // Ignore errors
        }
      }
      setProxyCheckResults(results);
    };
    if (profiles.length > 0) {
      void loadCachedResults();
    }
  }, [profiles]);

  const loadAllTags = React.useCallback(async () => {
    try {
      const tags = await invoke<string[]>("get_all_tags");
      setAllTags(tags);
    } catch (error) {
      console.error("Failed to load tags:", error);
    }
  }, []);

  const handleProxySelection = React.useCallback(
    async (profileId: string, proxyId: string | null) => {
      try {
        await invoke("update_profile_proxy", {
          profileId,
          proxyId,
        });
        setProxyOverrides((prev) => ({ ...prev, [profileId]: proxyId }));
        setVpnOverrides((prev) => ({ ...prev, [profileId]: null }));
        await emit("profile-updated");
      } catch (error) {
        console.error("Failed to update proxy settings:", error);
      } finally {
        setOpenProxySelectorFor(null);
      }
    },
    [],
  );

  const handleVpnSelection = React.useCallback(
    async (profileId: string, vpnId: string | null) => {
      try {
        await invoke("update_profile_vpn", {
          profileId,
          vpnId,
        });
        setVpnOverrides((prev) => ({ ...prev, [profileId]: vpnId }));
        setProxyOverrides((prev) => ({ ...prev, [profileId]: null }));
        await emit("profile-updated");
      } catch (error) {
        console.error("Failed to update VPN settings:", error);
      } finally {
        setOpenProxySelectorFor(null);
      }
    },
    [],
  );

  const handleCreateCountryProxy = React.useCallback(
    async (profileId: string, country: LocationItem) => {
      try {
        await invoke("create_cloud_location_proxy", {
          name: country.name,
          country: country.code,
          state: null,
          city: null,
        });
        await emit("stored-proxies-changed");
        // Wait briefly for proxy list to update, then find and assign the new proxy
        await new Promise((r) => setTimeout(r, 200));
        const updatedProxies =
          await invoke<StoredProxy[]>("get_stored_proxies");
        const newProxy = updatedProxies.find(
          (p: StoredProxy) =>
            p.is_cloud_derived && p.geo_country === country.code,
        );
        if (newProxy) {
          await handleProxySelection(profileId, newProxy.id);
        }
        setOpenProxySelectorFor(null);
      } catch (error) {
        console.error("Failed to create country proxy:", error);
      }
    },
    [handleProxySelection],
  );

  // Use shared browser state hook
  const browserState = useBrowserState(
    profiles,
    runningProfiles,
    isUpdating,
    launchingProfiles,
    stoppingProfiles,
  );

  // Listen for sync status events
  React.useEffect(() => {
    if (!browserState.isClient) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        unlisten = await listen<{
          profile_id: string;
          status: string;
          error?: string;
        }>("profile-sync-status", (event) => {
          const { profile_id, status, error } = event.payload;
          setSyncStatuses((prev) => ({
            ...prev,
            [profile_id]: { status, error },
          }));
        });
      } catch (error) {
        console.error("Failed to listen for sync status events:", error);
      }
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, [browserState.isClient]);

  // Fetch traffic snapshots for running profiles (lightweight, real-time data)
  // Convert Set to sorted array to avoid Set reference comparison issues in dependencies
  const runningProfileIds = React.useMemo(
    () => Array.from(runningProfiles).sort(),
    [runningProfiles],
  );
  const runningCount = runningProfileIds.length;
  React.useEffect(() => {
    if (!browserState.isClient) return;

    if (runningCount === 0) {
      setTrafficSnapshots({});
      return;
    }

    const fetchTrafficSnapshots = async () => {
      try {
        const allSnapshots = await invoke<TrafficSnapshot[]>(
          "get_all_traffic_snapshots",
        );
        const newSnapshots: Record<string, TrafficSnapshot> = {};
        for (const snapshot of allSnapshots) {
          if (snapshot.profile_id) {
            // Only keep snapshots for profiles that are currently running
            if (runningProfileIds.includes(snapshot.profile_id)) {
              const existing = newSnapshots[snapshot.profile_id];
              if (!existing || snapshot.last_update > existing.last_update) {
                newSnapshots[snapshot.profile_id] = snapshot;
              }
            }
          }
        }
        setTrafficSnapshots(newSnapshots);
      } catch (error) {
        console.error("Failed to fetch traffic snapshots:", error);
      }
    };

    void fetchTrafficSnapshots();
    const interval = setInterval(fetchTrafficSnapshots, 1000);
    return () => clearInterval(interval);
  }, [browserState.isClient, runningCount, runningProfileIds]);

  // Clean up snapshots for profiles that are no longer running
  React.useEffect(() => {
    if (!browserState.isClient) return;

    setTrafficSnapshots((prev) => {
      const cleaned: Record<string, TrafficSnapshot> = {};
      for (const [profileId, snapshot] of Object.entries(prev)) {
        // Only keep snapshots for profiles that are currently running
        if (runningProfileIds.includes(profileId)) {
          cleaned[profileId] = snapshot;
        }
      }
      // Only update if something was removed
      if (Object.keys(cleaned).length !== Object.keys(prev).length) {
        return cleaned;
      }
      return prev;
    });
  }, [browserState.isClient, runningProfileIds]);

  // Clear launching/stopping spinners when backend reports running status changes
  React.useEffect(() => {
    if (!browserState.isClient) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        unlisten = await listen<{ id: string; is_running: boolean }>(
          "profile-running-changed",
          (event) => {
            const { id } = event.payload;
            // Clear launching state for this profile if present
            setLaunchingProfiles((prev) => {
              if (!prev.has(id)) return prev;
              const next = new Set(prev);
              next.delete(id);
              return next;
            });
            // Clear stopping state for this profile if present
            setStoppingProfiles((prev) => {
              if (!prev.has(id)) return prev;
              const next = new Set(prev);
              next.delete(id);
              return next;
            });
          },
        );
      } catch (error) {
        console.error("Failed to listen for profile running changes:", error);
      }
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, [browserState.isClient]);

  // Keep stored proxies up-to-date by listening for changes emitted elsewhere in the app
  React.useEffect(() => {
    if (!browserState.isClient) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        unlisten = await listen("stored-proxies-changed", () => {
          // Also refresh tags on profile updates
          void loadAllTags();
        });
      } catch (_err) {
        // Best-effort only
      }
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, [browserState.isClient, loadAllTags]);

  // Automatically deselect profiles that become running, updating, launching, or stopping
  React.useEffect(() => {
    const newSet = new Set(selectedProfiles);
    let hasChanges = false;

    for (const profileId of selectedProfiles) {
      const profile = profiles.find((p) => p.id === profileId);
      if (profile) {
        const isRunning =
          browserState.isClient && runningProfiles.has(profile.id);
        const isLaunching = launchingProfiles.has(profile.id);
        const isStopping = stoppingProfiles.has(profile.id);
        const isBrowserUpdating = isUpdating(profile.browser);

        if (isRunning || isLaunching || isStopping || isBrowserUpdating) {
          newSet.delete(profileId);
          hasChanges = true;
        }
      }
    }

    if (hasChanges) {
      onSelectedProfilesChange(Array.from(newSet));
    }
  }, [
    profiles,
    runningProfiles,
    launchingProfiles,
    stoppingProfiles,
    isUpdating,
    browserState.isClient,
    onSelectedProfilesChange,
    selectedProfiles,
  ]);

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
      await onRenameProfile(profileToRename.id, newProfileName.trim());
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
    // Minimum loading time for visual feedback
    const minLoadingTime = new Promise((r) => setTimeout(r, 300));
    try {
      await Promise.all([onDeleteProfile(profileToDelete), minLoadingTime]);
      setProfileToDelete(null);
    } catch (error) {
      console.error("Failed to delete profile:", error);
    } finally {
      setIsDeleting(false);
    }
  };

  // Handle icon/checkbox click
  const handleIconClick = React.useCallback(
    (profileId: string) => {
      const profile = profiles.find((p) => p.id === profileId);
      if (!profile) return;

      // Prevent selection of profiles whose browsers are updating
      if (!browserState.canSelectProfile(profile)) {
        return;
      }

      setShowCheckboxes(true);
      const newSet = new Set(selectedProfiles);
      if (newSet.has(profileId)) {
        newSet.delete(profileId);
      } else {
        newSet.add(profileId);
      }

      // Hide checkboxes if no profiles are selected
      if (newSet.size === 0) {
        setShowCheckboxes(false);
      }

      onSelectedProfilesChange(Array.from(newSet));
    },
    [profiles, browserState, onSelectedProfilesChange, selectedProfiles],
  );

  React.useEffect(() => {
    if (browserState.isClient) {
      void loadAllTags();
    }
  }, [browserState.isClient, loadAllTags]);

  // Handle checkbox change
  const handleCheckboxChange = React.useCallback(
    (profileId: string, checked: boolean) => {
      const newSet = new Set(selectedProfiles);
      if (checked) {
        newSet.add(profileId);
      } else {
        newSet.delete(profileId);
      }

      // Hide checkboxes if no profiles are selected
      if (newSet.size === 0) {
        setShowCheckboxes(false);
      }

      onSelectedProfilesChange(Array.from(newSet));
    },
    [onSelectedProfilesChange, selectedProfiles],
  );

  // Handle select all checkbox
  const handleToggleAll = React.useCallback(
    (checked: boolean) => {
      const newSet = checked
        ? new Set(
            profiles
              .filter((profile) => {
                const isRunning =
                  browserState.isClient && runningProfiles.has(profile.id);
                const isLaunching = launchingProfiles.has(profile.id);
                const isStopping = stoppingProfiles.has(profile.id);
                const isBrowserUpdating = isUpdating(profile.browser);
                return (
                  !isRunning &&
                  !isLaunching &&
                  !isStopping &&
                  !isBrowserUpdating
                );
              })
              .map((profile) => profile.id),
          )
        : new Set<string>();

      setShowCheckboxes(checked);
      onSelectedProfilesChange(Array.from(newSet));
    },
    [
      profiles,
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
    return profiles.filter((profile) => {
      const isRunning =
        browserState.isClient && runningProfiles.has(profile.id);
      const isLaunching = launchingProfiles.has(profile.id);
      const isStopping = stoppingProfiles.has(profile.id);
      const isBrowserUpdating = isUpdating(profile.browser);
      return !isRunning && !isLaunching && !isStopping && !isBrowserUpdating;
    });
  }, [
    profiles,
    browserState.isClient,
    runningProfiles,
    launchingProfiles,
    stoppingProfiles,
    isUpdating,
  ]);

  // Build table meta from volatile state so columns can stay stable
  const tableMeta = React.useMemo<TableMeta>(
    () => ({
      selectedProfiles,
      selectableCount: selectableProfiles.length,
      showCheckboxes,
      isClient: browserState.isClient,
      runningProfiles,
      launchingProfiles,
      stoppingProfiles,
      isUpdating,
      browserState,

      // Tags editor state
      tagsOverrides,
      allTags,
      openTagsEditorFor,
      setAllTags,
      setOpenTagsEditorFor,
      setTagsOverrides,

      // Note editor state
      noteOverrides,
      openNoteEditorFor,
      setOpenNoteEditorFor,
      setNoteOverrides,

      // Proxy selector state
      openProxySelectorFor,
      setOpenProxySelectorFor,
      proxyOverrides,
      storedProxies,
      handleProxySelection,
      checkingProfileId,
      proxyCheckResults,

      // VPN selector state
      vpnConfigs,
      vpnOverrides,
      handleVpnSelection,

      // Selection helpers
      isProfileSelected: (id: string) => selectedProfiles.includes(id),
      handleToggleAll,
      handleCheckboxChange,
      handleIconClick,

      // Rename helpers
      handleRename,
      setProfileToRename,
      setNewProfileName,
      setRenameError,
      profileToRename,
      newProfileName,
      isRenamingSaving,
      renameError,

      // Launch/stop helpers
      setLaunchingProfiles,
      setStoppingProfiles,
      onKillProfile,
      onLaunchProfile,

      // Overflow actions
      onAssignProfilesToGroup,
      onCloneProfile,
      onConfigureCamoufox,
      onCopyCookiesToProfile,
      onImportCookies,
      onExportCookies,

      // Traffic snapshots (lightweight real-time data)
      trafficSnapshots,
      onOpenTrafficDialog: (profileId: string) => {
        const profile = profiles.find((p) => p.id === profileId);
        setTrafficDialogProfile({ id: profileId, name: profile?.name });
      },

      // Sync
      syncStatuses,
      onOpenProfileSyncDialog,
      onToggleProfileSync,
      crossOsUnlocked,
      syncUnlocked,

      // Country proxy creation
      countries,
      canCreateLocationProxy,
      loadCountries,
      handleCreateCountryProxy,
    }),
    [
      selectedProfiles,
      selectableProfiles.length,
      showCheckboxes,
      browserState.isClient,
      runningProfiles,
      launchingProfiles,
      stoppingProfiles,
      isUpdating,
      browserState,
      tagsOverrides,
      allTags,
      openTagsEditorFor,
      noteOverrides,
      openNoteEditorFor,
      openProxySelectorFor,
      proxyOverrides,
      storedProxies,
      handleProxySelection,
      checkingProfileId,
      proxyCheckResults,
      vpnConfigs,
      vpnOverrides,
      handleVpnSelection,
      handleToggleAll,
      handleCheckboxChange,
      handleIconClick,
      handleRename,
      profileToRename,
      newProfileName,
      isRenamingSaving,
      trafficSnapshots,
      profiles,
      renameError,
      onKillProfile,
      onLaunchProfile,
      onAssignProfilesToGroup,
      onCloneProfile,
      onConfigureCamoufox,
      onCopyCookiesToProfile,
      onImportCookies,
      onExportCookies,
      syncStatuses,
      onOpenProfileSyncDialog,
      onToggleProfileSync,
      crossOsUnlocked,
      syncUnlocked,
      countries,
      canCreateLocationProxy,
      loadCountries,
      handleCreateCountryProxy,
    ],
  );

  const columns: ColumnDef<BrowserProfile>[] = React.useMemo(
    () => [
      {
        id: "select",
        header: ({ table }) => {
          const meta = table.options.meta as TableMeta;
          return (
            <span>
              <Checkbox
                checked={
                  meta.selectedProfiles.length === meta.selectableCount &&
                  meta.selectableCount !== 0
                }
                onCheckedChange={(value) => meta.handleToggleAll(!!value)}
                aria-label="Select all"
                className="cursor-pointer"
              />
            </span>
          );
        },
        cell: ({ row, table }) => {
          const meta = table.options.meta as TableMeta;
          const profile = row.original;
          const browser = profile.browser;
          const IconComponent = getBrowserIcon(browser);
          const isCrossOs = isCrossOsProfile(profile);

          const isSelected = meta.isProfileSelected(profile.id);
          const isRunning =
            meta.isClient && meta.runningProfiles.has(profile.id);
          const isLaunching = meta.launchingProfiles.has(profile.id);
          const isStopping = meta.stoppingProfiles.has(profile.id);
          const isBrowserUpdating = meta.isUpdating(browser);
          const isDisabled =
            isRunning || isLaunching || isStopping || isBrowserUpdating;

          // Cross-OS profiles: show OS icon when checkboxes aren't visible, show checkbox when they are
          if (isCrossOs && !meta.showCheckboxes && !isSelected) {
            const osName = profile.host_os
              ? getOSDisplayName(profile.host_os)
              : "another OS";
            const OsIcon =
              profile.host_os === "macos"
                ? FaApple
                : profile.host_os === "windows"
                  ? FaWindows
                  : FaLinux;
            return (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="flex justify-center items-center w-4 h-4">
                    <button
                      type="button"
                      className="flex justify-center items-center p-0 border-none cursor-pointer"
                      onClick={() => meta.handleIconClick(profile.id)}
                      aria-label="Select profile"
                    >
                      <span className="w-4 h-4 group">
                        <OsIcon className="w-4 h-4 text-muted-foreground group-hover:hidden" />
                        <span className="peer border-input dark:bg-input/30 dark:data-[state=checked]:bg-primary size-4 shrink-0 rounded-[4px] border shadow-xs transition-shadow outline-none w-4 h-4 hidden group-hover:block pointer-events-none items-center justify-center duration-200" />
                      </span>
                    </button>
                  </span>
                </TooltipTrigger>
                <TooltipContent>
                  <p>
                    This profile was created on {osName} and is not supported on
                    this system
                  </p>
                </TooltipContent>
              </Tooltip>
            );
          }

          // Cross-OS profiles with checkboxes visible: show checkbox (selectable for bulk delete)
          if (isCrossOs && (meta.showCheckboxes || isSelected)) {
            const osName = profile.host_os
              ? getOSDisplayName(profile.host_os)
              : "another OS";
            return (
              <NonHoverableTooltip
                content={
                  <p>
                    This profile was created on {osName} and is not supported on
                    this system
                  </p>
                }
                sideOffset={4}
                horizontalOffset={8}
              >
                <span className="flex justify-center items-center w-4 h-4">
                  <Checkbox
                    checked={isSelected}
                    onCheckedChange={(value) =>
                      meta.handleCheckboxChange(profile.id, !!value)
                    }
                    aria-label="Select row"
                    className="w-4 h-4"
                  />
                </span>
              </NonHoverableTooltip>
            );
          }

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

          const browserName = getBrowserDisplayName(browser);

          if (meta.showCheckboxes || isSelected) {
            return (
              <NonHoverableTooltip
                content={<p>{browserName}</p>}
                sideOffset={4}
                horizontalOffset={8}
              >
                <span className="flex justify-center items-center w-4 h-4">
                  <Checkbox
                    checked={isSelected}
                    onCheckedChange={(value) =>
                      meta.handleCheckboxChange(profile.id, !!value)
                    }
                    aria-label="Select row"
                    className="w-4 h-4"
                  />
                </span>
              </NonHoverableTooltip>
            );
          }

          return (
            <NonHoverableTooltip
              content={<p>{browserName}</p>}
              sideOffset={4}
              horizontalOffset={8}
            >
              <span className="flex relative justify-center items-center w-4 h-4">
                <button
                  type="button"
                  className="flex justify-center items-center p-0 border-none cursor-pointer"
                  onClick={() => meta.handleIconClick(profile.id)}
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
            </NonHoverableTooltip>
          );
        },
        enableSorting: false,
        enableHiding: false,
        size: 40,
      },
      {
        id: "actions",
        cell: ({ row, table }) => {
          const meta = table.options.meta as TableMeta;
          const profile = row.original;
          const isRunning =
            meta.isClient && meta.runningProfiles.has(profile.id);
          const isLaunching = meta.launchingProfiles.has(profile.id);
          const isStopping = meta.stoppingProfiles.has(profile.id);
          const canLaunch = meta.browserState.canLaunchProfile(profile);
          const tooltipContent =
            meta.browserState.getLaunchTooltipContent(profile);

          const handleProfileStop = async (profile: BrowserProfile) => {
            meta.setStoppingProfiles((prev: Set<string>) =>
              new Set(prev).add(profile.id),
            );
            try {
              await meta.onKillProfile(profile);
            } catch (error) {
              meta.setStoppingProfiles((prev: Set<string>) => {
                const next = new Set(prev);
                next.delete(profile.id);
                return next;
              });
              throw error;
            }
          };

          const handleProfileLaunch = async (profile: BrowserProfile) => {
            meta.setLaunchingProfiles((prev: Set<string>) =>
              new Set(prev).add(profile.id),
            );
            try {
              await meta.onLaunchProfile(profile);
            } catch (error) {
              meta.setLaunchingProfiles((prev: Set<string>) => {
                const next = new Set(prev);
                next.delete(profile.id);
                return next;
              });
              throw error;
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
                        "min-w-[70px] h-7",
                        !canLaunch && "opacity-50 cursor-not-allowed",
                        canLaunch && "cursor-pointer",
                      )}
                      onClick={() =>
                        isRunning
                          ? handleProfileStop(profile)
                          : handleProfileLaunch(profile)
                      }
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
          const meta = table.options.meta as TableMeta;
          const profile = row.original as BrowserProfile;
          const rawName: string = row.getValue("name");
          const name = getBrowserDisplayName(rawName);
          const isEditing = meta.profileToRename?.id === profile.id;

          if (isEditing) {
            return (
              <div
                ref={renameContainerRef}
                className="overflow-visible relative"
              >
                <Input
                  autoFocus
                  value={meta.newProfileName}
                  onChange={(e) => {
                    meta.setNewProfileName(e.target.value);
                    if (meta.renameError) meta.setRenameError(null);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !(e.metaKey || e.ctrlKey)) {
                      void meta.handleRename();
                    } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                      void meta.handleRename();
                    } else if (e.key === "Escape") {
                      meta.setProfileToRename(null);
                      meta.setNewProfileName("");
                      meta.setRenameError(null);
                    }
                  }}
                  onBlur={() => {
                    if (
                      meta.newProfileName.trim().length > 0 &&
                      meta.newProfileName.trim() !== profile.name
                    ) {
                      void meta.handleRename();
                    } else {
                      meta.setProfileToRename(null);
                      meta.setNewProfileName("");
                      meta.setRenameError(null);
                    }
                  }}
                  className="w-30 h-6 px-2 py-1 text-sm font-medium leading-none border-0 shadow-none focus-visible:ring-0"
                />
              </div>
            );
          }

          const display =
            name.length < 14 ? (
              <div className="font-medium text-left leading-none">{name}</div>
            ) : (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="leading-none">{trimName(name, 14)}</span>
                </TooltipTrigger>
                <TooltipContent>{name}</TooltipContent>
              </Tooltip>
            );

          const isCrossOs = isCrossOsProfile(profile);
          const isRunning =
            meta.isClient && meta.runningProfiles.has(profile.id);
          const isLaunching = meta.launchingProfiles.has(profile.id);
          const isStopping = meta.stoppingProfiles.has(profile.id);
          const isBrowserUpdating = meta.isUpdating(profile.browser);
          const isDisabled =
            isRunning ||
            isLaunching ||
            isStopping ||
            isBrowserUpdating ||
            isCrossOs;

          return (
            <button
              type="button"
              className={cn(
                "px-2 py-1 mr-auto text-left bg-transparent rounded border-none w-30 h-6",
                isDisabled
                  ? "opacity-60 cursor-not-allowed"
                  : "cursor-pointer hover:bg-accent/50",
              )}
              onClick={() => {
                if (isDisabled) return;
                meta.setProfileToRename(profile);
                meta.setNewProfileName(profile.name);
                meta.setRenameError(null);
              }}
              onKeyDown={(e) => {
                if (isDisabled) return;
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  meta.setProfileToRename(profile);
                  meta.setNewProfileName(profile.name);
                  meta.setRenameError(null);
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
          const meta = table.options.meta as TableMeta;
          const profile = row.original;
          const isCrossOs = isCrossOsProfile(profile);
          const isRunning =
            meta.isClient && meta.runningProfiles.has(profile.id);
          const isLaunching = meta.launchingProfiles.has(profile.id);
          const isStopping = meta.stoppingProfiles.has(profile.id);
          const isBrowserUpdating = meta.isUpdating(profile.browser);
          const isDisabled =
            isRunning ||
            isLaunching ||
            isStopping ||
            isBrowserUpdating ||
            isCrossOs;

          return (
            <TagsCell
              profile={profile}
              isDisabled={isDisabled}
              tagsOverrides={meta.tagsOverrides || {}}
              allTags={meta.allTags || []}
              setAllTags={meta.setAllTags}
              openTagsEditorFor={meta.openTagsEditorFor || null}
              setOpenTagsEditorFor={meta.setOpenTagsEditorFor}
              setTagsOverrides={meta.setTagsOverrides}
            />
          );
        },
      },
      {
        id: "note",
        header: "Note",
        cell: ({ row, table }) => {
          const meta = table.options.meta as TableMeta;
          const profile = row.original;
          const isCrossOs = isCrossOsProfile(profile);
          const isRunning =
            meta.isClient && meta.runningProfiles.has(profile.id);
          const isLaunching = meta.launchingProfiles.has(profile.id);
          const isStopping = meta.stoppingProfiles.has(profile.id);
          const isBrowserUpdating = meta.isUpdating(profile.browser);
          const isDisabled =
            isRunning ||
            isLaunching ||
            isStopping ||
            isBrowserUpdating ||
            isCrossOs;

          return (
            <NoteCell
              profile={profile}
              isDisabled={isDisabled}
              noteOverrides={meta.noteOverrides || {}}
              openNoteEditorFor={meta.openNoteEditorFor || null}
              setOpenNoteEditorFor={meta.setOpenNoteEditorFor}
              setNoteOverrides={meta.setNoteOverrides}
            />
          );
        },
      },
      {
        id: "proxy",
        header: "Proxy / VPN",
        cell: ({ row, table }) => {
          const meta = table.options.meta as TableMeta;
          const profile = row.original;
          const isCrossOs = isCrossOsProfile(profile);
          const isRunning =
            meta.isClient && meta.runningProfiles.has(profile.id);
          const isLaunching = meta.launchingProfiles.has(profile.id);
          const isStopping = meta.stoppingProfiles.has(profile.id);
          const isBrowserUpdating = meta.isUpdating(profile.browser);
          const isDisabled =
            isRunning ||
            isLaunching ||
            isStopping ||
            isBrowserUpdating ||
            isCrossOs;

          const hasProxyOverride = Object.hasOwn(
            meta.proxyOverrides,
            profile.id,
          );
          const effectiveProxyId = hasProxyOverride
            ? meta.proxyOverrides[profile.id]
            : (profile.proxy_id ?? null);
          const effectiveProxy = effectiveProxyId
            ? (meta.storedProxies.find((p) => p.id === effectiveProxyId) ??
              null)
            : null;

          const hasVpnOverride = Object.hasOwn(meta.vpnOverrides, profile.id);
          const effectiveVpnId = hasVpnOverride
            ? meta.vpnOverrides[profile.id]
            : (profile.vpn_id ?? null);
          const effectiveVpn = effectiveVpnId
            ? (meta.vpnConfigs.find((v) => v.id === effectiveVpnId) ?? null)
            : null;

          const hasAssignment = Boolean(effectiveProxy || effectiveVpn);
          const displayName = effectiveVpn
            ? effectiveVpn.name
            : effectiveProxy
              ? effectiveProxy.name
              : "Not Selected";
          const vpnBadge = effectiveVpn
            ? effectiveVpn.vpn_type === "WireGuard"
              ? "WG"
              : "OVPN"
            : null;
          const tooltipText = hasAssignment ? displayName : null;
          const isSelectorOpen = meta.openProxySelectorFor === profile.id;
          const selectedId = effectiveVpnId ?? effectiveProxyId ?? null;

          // When profile is running, show bandwidth chart instead of proxy selector
          if (isRunning && meta.trafficSnapshots) {
            const snapshot = meta.trafficSnapshots[profile.id];
            const bandwidthData = snapshot?.recent_bandwidth
              ? [...snapshot.recent_bandwidth]
              : [];
            const currentBandwidth =
              (snapshot?.current_bytes_sent || 0) +
              (snapshot?.current_bytes_received || 0);

            return (
              <BandwidthMiniChart
                key={`${profile.id}-${snapshot?.last_update || 0}-${bandwidthData.length}`}
                data={bandwidthData}
                currentBandwidth={currentBandwidth}
                onClick={() => meta.onOpenTrafficDialog?.(profile.id)}
              />
            );
          }

          return (
            <div className="flex gap-2 items-center">
              <Popover
                open={isSelectorOpen}
                onOpenChange={(open) =>
                  meta.setOpenProxySelectorFor(open ? profile.id : null)
                }
              >
                <Tooltip>
                  <TooltipTrigger asChild>
                    <PopoverTrigger asChild>
                      <span
                        className={cn(
                          "flex gap-2 items-center px-2 py-1 rounded",
                          isDisabled
                            ? "opacity-60 cursor-not-allowed pointer-events-none"
                            : "cursor-pointer hover:bg-accent/50",
                        )}
                      >
                        {vpnBadge && (
                          <Badge
                            variant="outline"
                            className="text-[10px] px-1 py-0 leading-tight"
                          >
                            {vpnBadge}
                          </Badge>
                        )}
                        <span
                          className={cn(
                            "text-sm",
                            !hasAssignment && "text-muted-foreground",
                          )}
                        >
                          {hasAssignment
                            ? trimName(displayName, 10)
                            : displayName}
                        </span>
                      </span>
                    </PopoverTrigger>
                  </TooltipTrigger>
                  {tooltipText && (
                    <TooltipContent>{tooltipText}</TooltipContent>
                  )}
                </Tooltip>

                {!isDisabled && (
                  <PopoverContent
                    className="w-[240px] p-0"
                    align="end"
                    sideOffset={8}
                  >
                    <Command>
                      <CommandInput
                        placeholder={
                          meta.canCreateLocationProxy
                            ? "Search proxies, VPNs, or countries..."
                            : "Search proxies or VPNs..."
                        }
                        onFocus={() => {
                          if (meta.canCreateLocationProxy)
                            void meta.loadCountries();
                        }}
                      />
                      <CommandList>
                        <CommandEmpty>No proxies or VPNs found.</CommandEmpty>
                        <CommandGroup>
                          <CommandItem
                            value="__none__"
                            onSelect={() =>
                              void meta.handleProxySelection(profile.id, null)
                            }
                          >
                            <LuCheck
                              className={cn(
                                "mr-2 h-4 w-4",
                                selectedId === null
                                  ? "opacity-100"
                                  : "opacity-0",
                              )}
                            />
                            None
                          </CommandItem>
                          {meta.storedProxies.map((proxy) => (
                            <CommandItem
                              key={proxy.id}
                              value={proxy.name}
                              onSelect={() =>
                                void meta.handleProxySelection(
                                  profile.id,
                                  proxy.id,
                                )
                              }
                            >
                              <LuCheck
                                className={cn(
                                  "mr-2 h-4 w-4",
                                  effectiveProxyId === proxy.id && !effectiveVpn
                                    ? "opacity-100"
                                    : "opacity-0",
                                )}
                              />
                              {proxy.name}
                            </CommandItem>
                          ))}
                        </CommandGroup>
                        {meta.vpnConfigs.length > 0 && (
                          <CommandGroup heading="VPNs">
                            {meta.vpnConfigs.map((vpn) => (
                              <CommandItem
                                key={vpn.id}
                                value={`vpn-${vpn.name}`}
                                onSelect={() =>
                                  void meta.handleVpnSelection(
                                    profile.id,
                                    vpn.id,
                                  )
                                }
                              >
                                <LuCheck
                                  className={cn(
                                    "mr-2 h-4 w-4",
                                    effectiveVpnId === vpn.id
                                      ? "opacity-100"
                                      : "opacity-0",
                                  )}
                                />
                                <Badge
                                  variant="outline"
                                  className="text-[10px] px-1 py-0 leading-tight mr-1"
                                >
                                  {vpn.vpn_type === "WireGuard" ? "WG" : "OVPN"}
                                </Badge>
                                {vpn.name}
                              </CommandItem>
                            ))}
                          </CommandGroup>
                        )}
                        {meta.canCreateLocationProxy &&
                          meta.countries.length > 0 && (
                            <CommandGroup heading="Create by country">
                              {meta.countries
                                .filter(
                                  (c) =>
                                    !meta.storedProxies.some(
                                      (p) =>
                                        p.is_cloud_derived &&
                                        p.geo_country === c.code,
                                    ),
                                )
                                .map((country) => (
                                  <CommandItem
                                    key={`country-${country.code}`}
                                    value={`create-${country.name}`}
                                    onSelect={() =>
                                      void meta.handleCreateCountryProxy(
                                        profile.id,
                                        country,
                                      )
                                    }
                                  >
                                    <span className="mr-2 h-4 w-4" />+{" "}
                                    {country.name}
                                  </CommandItem>
                                ))}
                            </CommandGroup>
                          )}
                      </CommandList>
                    </Command>
                  </PopoverContent>
                )}
              </Popover>
              {effectiveProxy && !effectiveVpn && !isDisabled && (
                <ProxyCheckButton
                  proxy={effectiveProxy}
                  profileId={profile.id}
                  checkingProfileId={meta.checkingProfileId}
                  cachedResult={meta.proxyCheckResults[effectiveProxy.id]}
                  setCheckingProfileId={setCheckingProfileId}
                  onCheckComplete={(result) => {
                    setProxyCheckResults((prev) => ({
                      ...prev,
                      [effectiveProxy.id]: result,
                    }));
                  }}
                  onCheckFailed={(result) => {
                    setProxyCheckResults((prev) => ({
                      ...prev,
                      [effectiveProxy.id]: result,
                    }));
                  }}
                />
              )}
            </div>
          );
        },
      },
      {
        id: "sync",
        header: "",
        size: 24,
        cell: ({ row, table }) => {
          const profile = row.original;
          const meta = table.options.meta as TableMeta;
          const syncEntry = meta.syncStatuses[profile.id];
          const liveStatus = syncEntry?.status as
            | "syncing"
            | "waiting"
            | "synced"
            | "error"
            | "disabled"
            | undefined;

          const dot = getProfileSyncStatusDot(
            profile,
            liveStatus,
            syncEntry?.error,
          );
          if (!dot) return null;

          return (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="flex justify-center items-center w-3 h-3">
                  <span
                    className={`w-2 h-2 rounded-full ${dot.color}${dot.animate ? " animate-pulse" : ""}`}
                  />
                </span>
              </TooltipTrigger>
              <TooltipContent>{dot.tooltip}</TooltipContent>
            </Tooltip>
          );
        },
      },
      {
        id: "settings",
        cell: ({ row, table }) => {
          const meta = table.options.meta as TableMeta;
          const profile = row.original;
          const isCrossOs = isCrossOsProfile(profile);
          const isRunning =
            meta.isClient && meta.runningProfiles.has(profile.id);
          const isBrowserUpdating =
            meta.isClient && meta.isUpdating(profile.browser);
          const isLaunching = meta.launchingProfiles.has(profile.id);
          const isStopping = meta.stoppingProfiles.has(profile.id);
          const isDisabled =
            isRunning ||
            isLaunching ||
            isStopping ||
            isBrowserUpdating ||
            isCrossOs;
          const isDeleteDisabled =
            isRunning || isLaunching || isStopping || isBrowserUpdating;

          return (
            <div className="flex justify-end items-center">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    variant="ghost"
                    className="p-0 w-8 h-8"
                    disabled={!meta.isClient}
                  >
                    <span className="sr-only">Open menu</span>
                    <IoEllipsisHorizontal className="w-4 h-4" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem
                    onClick={() => {
                      meta.onOpenTrafficDialog?.(profile.id);
                    }}
                    disabled={isCrossOs}
                  >
                    View Network
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    onClick={() => {
                      if (meta.syncUnlocked) {
                        meta.onToggleProfileSync?.(profile);
                      }
                    }}
                    disabled={!meta.syncUnlocked || isCrossOs}
                  >
                    <span className="flex items-center gap-2">
                      {profile.sync_enabled ? "Disable Sync" : "Enable Sync"}
                      {!meta.syncUnlocked && <ProBadge />}
                    </span>
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    onClick={() => {
                      meta.onAssignProfilesToGroup?.([profile.id]);
                    }}
                    disabled={isDisabled}
                  >
                    Assign to Group
                  </DropdownMenuItem>
                  {(profile.browser === "camoufox" ||
                    profile.browser === "wayfern") &&
                    meta.onConfigureCamoufox && (
                      <DropdownMenuItem
                        onClick={() => {
                          meta.onConfigureCamoufox?.(profile);
                        }}
                        disabled={isDisabled}
                      >
                        <span className="flex items-center gap-2">
                          Change Fingerprint
                          {!meta.crossOsUnlocked && <ProBadge />}
                        </span>
                      </DropdownMenuItem>
                    )}
                  {(profile.browser === "camoufox" ||
                    profile.browser === "wayfern") &&
                    meta.onCopyCookiesToProfile && (
                      <DropdownMenuItem
                        onClick={() => {
                          if (meta.crossOsUnlocked) {
                            meta.onCopyCookiesToProfile?.(profile);
                          }
                        }}
                        disabled={isDisabled || !meta.crossOsUnlocked}
                      >
                        <span className="flex items-center gap-2">
                          Copy Cookies to Profile
                          {!meta.crossOsUnlocked && <ProBadge />}
                        </span>
                      </DropdownMenuItem>
                    )}
                  {(profile.browser === "camoufox" ||
                    profile.browser === "wayfern") &&
                    meta.onImportCookies && (
                      <DropdownMenuItem
                        onClick={() => {
                          if (meta.crossOsUnlocked) {
                            meta.onImportCookies?.(profile);
                          }
                        }}
                        disabled={isDisabled || !meta.crossOsUnlocked}
                      >
                        <span className="flex items-center gap-2">
                          Import Cookies
                          {!meta.crossOsUnlocked && <ProBadge />}
                        </span>
                      </DropdownMenuItem>
                    )}
                  {(profile.browser === "camoufox" ||
                    profile.browser === "wayfern") &&
                    meta.onExportCookies && (
                      <DropdownMenuItem
                        onClick={() => {
                          if (meta.crossOsUnlocked) {
                            meta.onExportCookies?.(profile);
                          }
                        }}
                        disabled={isDisabled || !meta.crossOsUnlocked}
                      >
                        <span className="flex items-center gap-2">
                          Export Cookies
                          {!meta.crossOsUnlocked && <ProBadge />}
                        </span>
                      </DropdownMenuItem>
                    )}
                  <DropdownMenuItem
                    onClick={() => {
                      meta.onCloneProfile?.(profile);
                    }}
                    disabled={isDisabled}
                  >
                    Clone Profile
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    onClick={() => {
                      setProfileToDelete(profile);
                    }}
                    disabled={isDeleteDisabled}
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
    [],
  );

  const table = useReactTable({
    data: profiles,
    columns,
    state: {
      sorting,
      rowSelection,
    },
    onSortingChange: handleSortingChange,
    onRowSelectionChange: handleRowSelectionChange,
    enableRowSelection: (row) => {
      const profile = row.original;
      const isRunning =
        browserState.isClient && runningProfiles.has(profile.id);
      const isLaunching = launchingProfiles.has(profile.id);
      const isStopping = stoppingProfiles.has(profile.id);
      const isBrowserUpdating = isUpdating(profile.browser);
      return !isRunning && !isLaunching && !isStopping && !isBrowserUpdating;
    },
    getSortedRowModel: getSortedRowModel(),
    getCoreRowModel: getCoreRowModel(),
    getRowId: (row) => row.id,
    meta: tableMeta,
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
                  className={cn(
                    "overflow-visible hover:bg-accent/50",
                    isCrossOsProfile(row.original) && "opacity-60",
                  )}
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
      <DataTableActionBar table={table}>
        <DataTableActionBarSelection table={table} />
        {onBulkGroupAssignment && (
          <DataTableActionBarAction
            tooltip="Assign to Group"
            onClick={onBulkGroupAssignment}
            size="icon"
          >
            <LuUsers />
          </DataTableActionBarAction>
        )}
        {onBulkProxyAssignment && (
          <DataTableActionBarAction
            tooltip="Assign Proxy"
            onClick={onBulkProxyAssignment}
            size="icon"
          >
            <FiWifi />
          </DataTableActionBarAction>
        )}
        {onBulkCopyCookies && (
          <DataTableActionBarAction
            tooltip={crossOsUnlocked ? "Copy Cookies" : "Copy Cookies (Pro)"}
            onClick={onBulkCopyCookies}
            size="icon"
            disabled={!crossOsUnlocked}
          >
            <LuCookie />
          </DataTableActionBarAction>
        )}
        {onBulkDelete && (
          <DataTableActionBarAction
            tooltip="Delete"
            onClick={onBulkDelete}
            size="icon"
            variant="destructive"
            className="border-destructive bg-destructive/50 hover:bg-destructive/70"
          >
            <LuTrash2 />
          </DataTableActionBarAction>
        )}
      </DataTableActionBar>
      {trafficDialogProfile && (
        <TrafficDetailsDialog
          isOpen={trafficDialogProfile !== null}
          onClose={() => setTrafficDialogProfile(null)}
          profileId={trafficDialogProfile.id}
          profileName={trafficDialogProfile.name}
        />
      )}
    </>
  );
}
