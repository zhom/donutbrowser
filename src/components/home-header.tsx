"use client";

import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import { LuChevronLeft, LuChevronRight, LuSearch, LuX } from "react-icons/lu";
import { getCurrentOS } from "@/lib/browser-utils";
import { cn } from "@/lib/utils";
import type { GroupWithCount } from "@/types";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";

const HOLD_MS = 150;
const DRAG_THRESHOLD_PX = 3;

const isTextInputTarget = (target: EventTarget | null): boolean => {
  if (!(target instanceof Element)) return false;
  const el = target.closest(
    "input, select, textarea, [contenteditable=''], [contenteditable='true']",
  );
  return el !== null;
};

const ALL_FILTER_ID = "__all__";

interface Props {
  onCreateProfileDialogOpen: (open: boolean) => void;
  searchQuery: string;
  onSearchQueryChange: (query: string) => void;
  groups: GroupWithCount[];
  totalProfiles: number;
  selectedGroupId: string | null;
  onGroupSelect: (groupId: string) => void;
  pageTitle?: string;
}

const HomeHeader = ({
  onCreateProfileDialogOpen,
  searchQuery,
  onSearchQueryChange,
  groups,
  totalProfiles,
  selectedGroupId,
  onGroupSelect,
  pageTitle,
}: Props) => {
  const { t } = useTranslation();
  const [platform, setPlatform] = useState<string>("macos");

  useEffect(() => {
    setPlatform(getCurrentOS());
  }, []);

  const isMacOS = platform === "macos";
  const showProfileToolbar = !pageTitle;

  // Press-and-hold drag: any pixel of the sys-bar becomes a drag handle after
  // HOLD_MS, but quick clicks still reach buttons/inputs underneath.
  const holdTimeoutRef = useRef<number | null>(null);
  const dragStartRef = useRef<{ x: number; y: number } | null>(null);
  const dragStartedRef = useRef(false);
  const activePointerIdRef = useRef<number | null>(null);
  const dragRootRef = useRef<HTMLDivElement | null>(null);

  const clearHold = useCallback(() => {
    if (holdTimeoutRef.current !== null) {
      window.clearTimeout(holdTimeoutRef.current);
      holdTimeoutRef.current = null;
    }
  }, []);

  const beginDrag = useCallback(() => {
    if (dragStartedRef.current) return;
    dragStartedRef.current = true;
    clearHold();
    void getCurrentWindow().startDragging();
  }, [clearHold]);

  useEffect(() => {
    return () => {
      clearHold();
    };
  }, [clearHold]);

  const handlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (e.button !== 0) return;
      if (isTextInputTarget(e.target)) return;

      dragStartedRef.current = false;
      dragStartRef.current = { x: e.clientX, y: e.clientY };
      activePointerIdRef.current = e.pointerId;

      clearHold();
      holdTimeoutRef.current = window.setTimeout(() => {
        holdTimeoutRef.current = null;
        beginDrag();
      }, HOLD_MS);
    },
    [beginDrag, clearHold],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (
        dragStartedRef.current ||
        dragStartRef.current === null ||
        activePointerIdRef.current !== e.pointerId
      ) {
        return;
      }
      const dx = e.clientX - dragStartRef.current.x;
      const dy = e.clientY - dragStartRef.current.y;
      if (Math.hypot(dx, dy) > DRAG_THRESHOLD_PX) {
        beginDrag();
      }
    },
    [beginDrag],
  );

  const handlePointerEnd = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (activePointerIdRef.current !== e.pointerId) return;
      clearHold();
      dragStartRef.current = null;
      activePointerIdRef.current = null;
      dragStartedRef.current = false;
    },
    [clearHold],
  );

  const handleDoubleClick = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      if (isTextInputTarget(e.target)) return;
      if (e.target instanceof Element && e.target.closest("button")) return;
      clearHold();
      void getCurrentWindow().toggleMaximize();
    },
    [clearHold],
  );

  // Horizontal scroll fades for the group filter strip — when the user
  // has more groups than fit, the right edge fades to hint at overflow.
  const groupsScrollRef = useRef<HTMLDivElement | null>(null);
  const [groupsFadeLeft, setGroupsFadeLeft] = useState(false);
  const [groupsFadeRight, setGroupsFadeRight] = useState(false);
  useEffect(() => {
    const el = groupsScrollRef.current;
    if (!el) return;
    const update = () => {
      setGroupsFadeLeft(el.scrollLeft > 1);
      setGroupsFadeRight(el.scrollWidth - el.clientWidth - el.scrollLeft > 1);
    };
    update();
    el.addEventListener("scroll", update, { passive: true });
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => {
      el.removeEventListener("scroll", update);
      ro.disconnect();
    };
  }, []);

  const isWindows = platform === "windows";

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: titlebar drag surface; the interactive controls inside are real buttons/inputs
    <div
      ref={dragRootRef}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerEnd}
      onPointerCancel={handlePointerEnd}
      onDoubleClick={handleDoubleClick}
      className={cn(
        "flex h-11 items-center gap-2 border-b border-border bg-card pl-3 select-none",
        // Windows: WindowDragArea renders three 44px native-style controls
        // (minimize + maximize/restore + close) fixed at top-right with
        // z-50, total 132px wide. Reserve 144px on the right edge so the
        // "+ New" button and search input clear them with a few pixels of
        // breathing room and never sit underneath the controls.
        isWindows ? "pr-[144px]" : "pr-3",
      )}
    >
      {isMacOS && (
        <div
          aria-hidden="true"
          className="mr-1 flex shrink-0 items-center gap-[7px]"
        >
          {/* Reserve space for the macOS native traffic lights — the OS draws
              the colored buttons here through the transparent titlebar. */}
          <div className="size-[11px] rounded-full" />
          <div className="size-[11px] rounded-full" />
          <div className="size-[11px] rounded-full" />
        </div>
      )}

      {pageTitle ? (
        <span className="ml-2 text-xs font-semibold text-card-foreground">
          {pageTitle}
        </span>
      ) : null}

      {showProfileToolbar && (
        <div className="relative flex min-w-0 flex-1 items-center">
          {groupsFadeLeft && (
            <button
              type="button"
              aria-label={t("header.scrollGroupsLeft")}
              onClick={() => {
                const el = groupsScrollRef.current;
                if (el)
                  el.scrollBy({
                    left: -el.clientWidth * 0.6,
                    behavior: "smooth",
                  });
              }}
              className="absolute top-1/2 left-0 z-10 grid size-5 -translate-y-1/2 place-items-center rounded-full bg-card/90 text-muted-foreground shadow-sm transition-colors hover:bg-accent hover:text-foreground"
            >
              <LuChevronLeft className="size-3" />
            </button>
          )}
          <div
            ref={groupsScrollRef}
            className="ml-2 flex scrollbar-none items-center gap-3 overflow-x-auto scroll-smooth [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden"
            style={{
              paddingLeft: groupsFadeLeft ? 22 : 0,
              paddingRight: groupsFadeRight ? 22 : 0,
            }}
          >
            {/* "All" filter — shows every profile regardless of group. */}
            {(() => {
              const active = selectedGroupId === ALL_FILTER_ID;
              return (
                <button
                  key="__all__"
                  type="button"
                  onClick={() => {
                    onGroupSelect(ALL_FILTER_ID);
                  }}
                  className={cn(
                    "flex h-7 shrink-0 items-center gap-1.5 px-1 text-xs transition-colors duration-100",
                    active
                      ? "font-medium text-foreground"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  <span>{t("groups.all")}</span>
                  <span className="text-[11px] text-muted-foreground tabular-nums">
                    {totalProfiles}
                  </span>
                </button>
              );
            })()}
            {groups.map((group) => {
              const active = selectedGroupId === group.id;
              return (
                <button
                  key={group.id}
                  type="button"
                  title={group.name}
                  onClick={() => {
                    onGroupSelect(active ? ALL_FILTER_ID : group.id);
                  }}
                  className={cn(
                    "flex h-7 shrink-0 items-center gap-1.5 px-1 text-xs transition-colors duration-100",
                    active
                      ? "font-medium text-foreground"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  <span className="max-w-40 truncate">{group.name}</span>
                  <span className="text-[11px] text-muted-foreground tabular-nums">
                    {group.count}
                  </span>
                </button>
              );
            })}
          </div>
          {groupsFadeRight && (
            <button
              type="button"
              aria-label={t("header.scrollGroupsRight")}
              onClick={() => {
                const el = groupsScrollRef.current;
                if (el)
                  el.scrollBy({
                    left: el.clientWidth * 0.6,
                    behavior: "smooth",
                  });
              }}
              className="absolute top-1/2 right-0 z-10 grid size-5 -translate-y-1/2 place-items-center rounded-full bg-card/90 text-muted-foreground shadow-sm transition-colors hover:bg-accent hover:text-foreground"
            >
              <LuChevronRight className="size-3" />
            </button>
          )}
        </div>
      )}

      {!showProfileToolbar && <div className="flex-1" />}

      {showProfileToolbar && (
        <div className="relative shrink-0">
          <Input
            type="text"
            placeholder={t("header.searchPlaceholder")}
            value={searchQuery}
            onChange={(e) => {
              onSearchQueryChange(e.target.value);
            }}
            className="h-7 w-36 pr-7 pl-8 text-xs min-[860px]:w-52"
          />
          <LuSearch className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 transform text-muted-foreground" />
          {searchQuery ? (
            <button
              type="button"
              onClick={() => {
                onSearchQueryChange("");
              }}
              className="absolute top-1/2 right-1.5 -translate-y-1/2 transform rounded-sm p-0.5 transition-colors hover:bg-accent"
              aria-label={t("header.clearSearch")}
            >
              <LuX className="size-3.5 text-muted-foreground hover:text-foreground" />
            </button>
          ) : null}
        </div>
      )}

      {showProfileToolbar && (
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="shrink-0">
              <Button
                size="sm"
                data-onborda="create-profile"
                onClick={() => {
                  onCreateProfileDialogOpen(true);
                }}
                className="flex h-7 items-center gap-1.5 px-2.5 text-xs"
              >
                <GoPlus className="size-3.5" />
                {t("header.newProfile")}
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent>{t("header.createProfile")}</TooltipContent>
        </Tooltip>
      )}
    </div>
  );
};

export default HomeHeader;
