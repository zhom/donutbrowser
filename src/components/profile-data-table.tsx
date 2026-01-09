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
import { FiWifi } from "react-icons/fi";
import { IoEllipsisHorizontal } from "react-icons/io5";
import {
  LuCheck,
  LuChevronDown,
  LuChevronUp,
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
import {
  getBrowserDisplayName,
  getBrowserIcon,
  getCurrentOS,
} from "@/lib/browser-utils";
import { trimName } from "@/lib/name-utils";
import { cn } from "@/lib/utils";
import type {
  BrowserProfile,
  ProxyCheckResult,
  StoredProxy,
  TrafficSnapshot,
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

  // Traffic snapshots (lightweight real-time data)
  trafficSnapshots: Record<string, TrafficSnapshot>;
  onOpenTrafficDialog?: (profileId: string) => void;

  // Sync
  syncStatuses: Record<string, string>;
  onOpenProfileSyncDialog?: (profile: BrowserProfile) => void;
  onToggleProfileSync?: (profile: BrowserProfile) => void;
};

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
  onDeleteProfile: (profile: BrowserProfile) => void | Promise<void>;
  onRenameProfile: (profileId: string, newName: string) => Promise<void>;
  onConfigureCamoufox: (profile: BrowserProfile) => void;
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
  onOpenProfileSyncDialog?: (profile: BrowserProfile) => void;
  onToggleProfileSync?: (profile: BrowserProfile) => void;
}

export function ProfilesDataTable({
  profiles,
  onLaunchProfile,
  onKillProfile,
  onDeleteProfile,
  onRenameProfile,
  onConfigureCamoufox,
  runningProfiles,
  isUpdating,
  onAssignProfilesToGroup,
  selectedProfiles,
  onSelectedProfilesChange,
  onBulkDelete,
  onBulkGroupAssignment,
  onBulkProxyAssignment,
  onOpenProfileSyncDialog,
  onToggleProfileSync,
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

  const [proxyOverrides, setProxyOverrides] = React.useState<
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
    Record<string, string>
  >({});

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
        unlisten = await listen<{ profile_id: string; status: string }>(
          "profile-sync-status",
          (event) => {
            const { profile_id, status } = event.payload;
            setSyncStatuses((prev) => ({ ...prev, [profile_id]: status }));
          },
        );
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
    [
      profiles,
      browserState.canSelectProfile,
      onSelectedProfilesChange,
      selectedProfiles,
    ],
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
      onConfigureCamoufox,

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
      onConfigureCamoufox,
      syncStatuses,
      onOpenProfileSyncDialog,
      onToggleProfileSync,
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

          const isSelected = meta.isProfileSelected(profile.id);
          const isRunning =
            meta.isClient && meta.runningProfiles.has(profile.id);
          const isLaunching = meta.launchingProfiles.has(profile.id);
          const isStopping = meta.stoppingProfiles.has(profile.id);
          const isBrowserUpdating = meta.isUpdating(browser);
          const isDisabled =
            isRunning || isLaunching || isStopping || isBrowserUpdating;

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

          const isRunning =
            meta.isClient && meta.runningProfiles.has(profile.id);
          const isLaunching = meta.launchingProfiles.has(profile.id);
          const isStopping = meta.stoppingProfiles.has(profile.id);
          const isBrowserUpdating = meta.isUpdating(profile.browser);
          const isDisabled =
            isRunning || isLaunching || isStopping || isBrowserUpdating;

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
          const isRunning =
            meta.isClient && meta.runningProfiles.has(profile.id);
          const isLaunching = meta.launchingProfiles.has(profile.id);
          const isStopping = meta.stoppingProfiles.has(profile.id);
          const isBrowserUpdating = meta.isUpdating(profile.browser);
          const isDisabled =
            isRunning || isLaunching || isStopping || isBrowserUpdating;

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
          const isRunning =
            meta.isClient && meta.runningProfiles.has(profile.id);
          const isLaunching = meta.launchingProfiles.has(profile.id);
          const isStopping = meta.stoppingProfiles.has(profile.id);
          const isBrowserUpdating = meta.isUpdating(profile.browser);
          const isDisabled =
            isRunning || isLaunching || isStopping || isBrowserUpdating;

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
        header: "Proxy",
        cell: ({ row, table }) => {
          const meta = table.options.meta as TableMeta;
          const profile = row.original;
          const isRunning =
            meta.isClient && meta.runningProfiles.has(profile.id);
          const isLaunching = meta.launchingProfiles.has(profile.id);
          const isStopping = meta.stoppingProfiles.has(profile.id);
          const isBrowserUpdating = meta.isUpdating(profile.browser);
          const isDisabled =
            isRunning || isLaunching || isStopping || isBrowserUpdating;

          const hasOverride = Object.hasOwn(meta.proxyOverrides, profile.id);
          const effectiveProxyId = hasOverride
            ? meta.proxyOverrides[profile.id]
            : (profile.proxy_id ?? null);
          const effectiveProxy = effectiveProxyId
            ? (meta.storedProxies.find((p) => p.id === effectiveProxyId) ??
              null)
            : null;
          const displayName = effectiveProxy
            ? effectiveProxy.name
            : "Not Selected";
          const profileHasProxy = Boolean(effectiveProxy);
          const tooltipText =
            profileHasProxy && effectiveProxy ? effectiveProxy.name : null;
          const isSelectorOpen = meta.openProxySelectorFor === profile.id;

          // When profile is running, show bandwidth chart instead of proxy selector
          if (isRunning && meta.trafficSnapshots) {
            // Find the traffic snapshot for this profile by matching profile_id
            const snapshot = meta.trafficSnapshots[profile.id];
            // Only use recent_bandwidth (last 60 seconds) - minimal data needed for mini chart
            // Create a new array reference to ensure React detects changes
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
                        <span
                          className={cn(
                            "text-sm",
                            !profileHasProxy && "text-muted-foreground",
                          )}
                        >
                          {profileHasProxy
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
                      <CommandInput placeholder="Search proxies..." />
                      <CommandList>
                        <CommandEmpty>No proxies found.</CommandEmpty>
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
                                effectiveProxyId === null
                                  ? "opacity-100"
                                  : "opacity-0",
                              )}
                            />
                            No Proxy
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
              {profileHasProxy && effectiveProxy && !isDisabled && (
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
          const meta = table.options.meta as TableMeta;
          const profile = row.original;

          if (!profile.sync_enabled) {
            return (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="flex justify-center items-center w-3 h-3">
                    <span className="w-2 h-2 rounded-full bg-muted-foreground/30" />
                  </span>
                </TooltipTrigger>
                <TooltipContent>Sync disabled</TooltipContent>
              </Tooltip>
            );
          }

          const syncStatus = meta.syncStatuses[profile.id];
          const isSyncing = syncStatus === "syncing";
          const isWaiting = syncStatus === "waiting";
          const isSynced =
            syncStatus === "synced" || (!syncStatus && profile.last_sync);
          const isError = syncStatus === "error";

          let dotClass = "bg-yellow-500";
          let tooltipText = "Sync pending";

          if (isSyncing) {
            dotClass = "bg-yellow-500 animate-pulse";
            tooltipText = "Syncing...";
          } else if (isWaiting) {
            dotClass = "bg-yellow-500";
            tooltipText = "Waiting for profile to stop";
          } else if (isError) {
            dotClass = "bg-red-500";
            tooltipText = "Sync error";
          } else if (isSynced) {
            dotClass = "bg-green-500";
            tooltipText = profile.last_sync
              ? `Last synced: ${new Date(profile.last_sync * 1000).toLocaleString()}`
              : "Synced";
          }

          return (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="flex justify-center items-center w-3 h-3">
                  <span className={`w-2 h-2 rounded-full ${dotClass}`} />
                </span>
              </TooltipTrigger>
              <TooltipContent>{tooltipText}</TooltipContent>
            </Tooltip>
          );
        },
      },
      {
        id: "settings",
        cell: ({ row, table }) => {
          const meta = table.options.meta as TableMeta;
          const profile = row.original;
          const isRunning =
            meta.isClient && meta.runningProfiles.has(profile.id);
          const isBrowserUpdating =
            meta.isClient && meta.isUpdating(profile.browser);
          const isLaunching = meta.launchingProfiles.has(profile.id);
          const isStopping = meta.stoppingProfiles.has(profile.id);
          const isDisabled =
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
                  >
                    View Network
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
                      >
                        Change Fingerprint
                      </DropdownMenuItem>
                    )}
                  {meta.onOpenProfileSyncDialog && (
                    <DropdownMenuItem
                      onClick={() => {
                        meta.onOpenProfileSyncDialog?.(profile);
                      }}
                    >
                      Sync Settings
                    </DropdownMenuItem>
                  )}
                  {meta.onToggleProfileSync && (
                    <DropdownMenuItem
                      onClick={() => {
                        meta.onToggleProfileSync?.(profile);
                      }}
                      disabled={isDisabled}
                    >
                      {profile.sync_enabled ? "Disable Sync" : "Enable Sync"}
                    </DropdownMenuItem>
                  )}
                  <DropdownMenuItem
                    onClick={() => {
                      setProfileToDelete(profile);
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
