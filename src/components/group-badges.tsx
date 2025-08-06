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
  if (isLoading) {
    return (
      <div className="flex flex-wrap gap-2 mb-4">
        <div className="flex items-center gap-2 px-4.5 py-1.5 text-xs">
          Loading groups...
        </div>
      </div>
    );
  }

  if (groups.length === 0) {
    return null;
  }

  return (
    <div className="flex flex-wrap gap-2 mb-4">
      {groups.map((group) => (
        <Badge
          key={group.id}
          variant={selectedGroupId === group.id ? "default" : "secondary"}
          className="cursor-pointer hover:bg-primary/80 transition-colors flex items-center gap-2 px-3 py-1"
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
