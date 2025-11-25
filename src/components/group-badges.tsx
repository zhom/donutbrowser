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

  const checkScrollPosition = useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return;

    const { scrollLeft, scrollWidth, clientWidth } = container;
    setShowLeftFade(scrollLeft > 0);
    setShowRightFade(scrollLeft < scrollWidth - clientWidth - 1);
  }, []);

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
        className="flex gap-2 overflow-x-auto pb-2 -mb-2 [scrollbar-width:none] [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden"
        onScroll={checkScrollPosition}
      >
        {groups.map((group) => (
          <Badge
            key={group.id}
            variant={selectedGroupId === group.id ? "default" : "secondary"}
            className="flex gap-2 items-center px-3 py-1 transition-colors cursor-pointer dark:hover:bg-primary/60 hover:bg-primary/80 flex-shrink-0"
            onClick={() => {
              onGroupSelect(
                selectedGroupId === group.id ? "default" : group.id,
              );
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
