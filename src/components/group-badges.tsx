"use client";

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
  if (isLoading && !groups.length) {
    return (
      <div className="flex flex-wrap gap-2 mb-4">
        <div className="flex items-center gap-2 px-4.5 py-1.5 text-xs">
          Loading groups...
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-wrap gap-2 mb-4">
      {groups.map((group) => (
        <Badge
          key={group.id}
          variant={selectedGroupId === group.id ? "default" : "secondary"}
          className="flex gap-2 items-center px-3 py-1 transition-colors cursor-pointer dark:hover:bg-primary/60 hover:bg-primary/80"
          onClick={() => {
            onGroupSelect(selectedGroupId === group.id ? "default" : group.id);
          }}
        >
          <span>{group.name}</span>
          <span className="bg-background/20 text-xs px-1.5 py-0.5 rounded-sm">
            {group.count}
          </span>
        </Badge>
      ))}
    </div>
  );
}
