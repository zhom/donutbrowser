"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { Badge } from "@/components/ui/badge";
import type { GroupWithCount } from "@/types";

interface GroupBadgesProps {
  selectedGroupId: string | null;
  onGroupSelect: (groupId: string) => void;
  refreshTrigger?: number;
  groups: GroupWithCount[];
  isLoading: boolean;
}

export function GroupBadges({
  selectedGroupId,
  onGroupSelect,
  groups,
  isLoading,
}: GroupBadgesProps) {
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const [showLeftFade, setShowLeftFade] = useState(false);
  const [showRightFade, setShowRightFade] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const dragStartRef = useRef<{ x: number; scrollLeft: number } | null>(null);
  const hasMovedRef = useRef(false);
  const clickBlockedRef = useRef(false);

  const checkScrollPosition = useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return;

    const { scrollLeft, scrollWidth, clientWidth } = container;
    setShowLeftFade(scrollLeft > 0);
    setShowRightFade(scrollLeft < scrollWidth - clientWidth - 1);
  }, []);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    const container = scrollContainerRef.current;
    if (!container) return;

    dragStartRef.current = {
      x: e.clientX,
      scrollLeft: container.scrollLeft,
    };
    hasMovedRef.current = false;
    setIsDragging(true);
    container.style.cursor = "grabbing";
    container.style.userSelect = "none";
  }, []);

  const handleMouseMove = useCallback(
    (e: MouseEvent) => {
      if (!isDragging || !dragStartRef.current) return;

      const container = scrollContainerRef.current;
      if (!container) return;

      const deltaX = e.clientX - dragStartRef.current.x;
      const distance = Math.abs(deltaX);

      if (distance > 5) {
        hasMovedRef.current = true;
      }

      container.scrollLeft = dragStartRef.current.scrollLeft - deltaX;
      checkScrollPosition();
    },
    [isDragging, checkScrollPosition],
  );

  const handleMouseUp = useCallback(() => {
    if (!isDragging) return;

    const container = scrollContainerRef.current;
    if (container) {
      container.style.cursor = "";
      container.style.userSelect = "";
    }

    clickBlockedRef.current = hasMovedRef.current;
    setIsDragging(false);
    dragStartRef.current = null;

    setTimeout(() => {
      hasMovedRef.current = false;
      clickBlockedRef.current = false;
    }, 100);
  }, [isDragging]);

  useEffect(() => {
    if (isDragging) {
      document.addEventListener("mousemove", handleMouseMove);
      document.addEventListener("mouseup", handleMouseUp);
      return () => {
        document.removeEventListener("mousemove", handleMouseMove);
        document.removeEventListener("mouseup", handleMouseUp);
      };
    }
  }, [isDragging, handleMouseMove, handleMouseUp]);

  useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;

    checkScrollPosition();
    container.addEventListener("scroll", checkScrollPosition);
    const resizeObserver = new ResizeObserver(checkScrollPosition);
    resizeObserver.observe(container);

    return () => {
      container.removeEventListener("scroll", checkScrollPosition);
      resizeObserver.disconnect();
    };
  }, [checkScrollPosition]);

  if (isLoading && !groups.length) {
    return (
      <div className="flex gap-2 mb-4">
        <div className="flex items-center gap-2 px-4.5 py-1.5 text-xs">
          Loading groups...
        </div>
      </div>
    );
  }

  return (
    <div className="relative mb-4">
      {showLeftFade && (
        <div className="absolute left-0 top-0 bottom-0 w-8 bg-gradient-to-r from-background to-transparent pointer-events-none z-10" />
      )}
      {showRightFade && (
        <div className="absolute right-0 top-0 bottom-0 w-8 bg-gradient-to-l from-background to-transparent pointer-events-none z-10" />
      )}
      <div
        ref={scrollContainerRef}
        role="region"
        aria-label="Profile groups"
        className={`flex gap-2 overflow-x-auto pb-2 -mb-2 [scrollbar-width:none] [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden ${isDragging ? "cursor-grabbing" : "cursor-grab"}`}
        onScroll={checkScrollPosition}
        onMouseDown={handleMouseDown}
      >
        {groups.map((group) => (
          <Badge
            key={group.id}
            variant={selectedGroupId === group.id ? "default" : "secondary"}
            className="flex gap-2 items-center px-3 py-1 transition-colors cursor-pointer dark:hover:bg-primary/60 hover:bg-primary/80 flex-shrink-0"
            onClick={(e) => {
              if (hasMovedRef.current || clickBlockedRef.current) {
                e.preventDefault();
                e.stopPropagation();
                return;
              }
              onGroupSelect(
                selectedGroupId === group.id ? "default" : group.id,
              );
            }}
            onMouseDown={(e) => {
              if (isDragging) {
                e.preventDefault();
                e.stopPropagation();
              }
            }}
          >
            <span>{group.name}</span>
            <span className="bg-background/20 text-xs px-1.5 py-0.5 rounded-sm">
              {group.count}
            </span>
          </Badge>
        ))}
      </div>
    </div>
  );
}
