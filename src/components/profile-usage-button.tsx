"use client";

import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import type { BrowserProfile } from "@/types";

export function ProfileUsageButton({
  profiles,
  failed = false,
  label,
}: {
  profiles: BrowserProfile[] | null;
  failed?: boolean;
  label: string;
}) {
  const { t } = useTranslation();
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          data-slot="profile-usage"
          size="sm"
          variant="ghost"
          aria-label={label}
          className="h-7 min-w-7 px-1.5 tabular-nums"
        >
          {profiles?.length ?? "?"}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-64 space-y-2 p-3">
        <h3 className="break-words text-sm font-medium">{label}</h3>
        {profiles?.length ? (
          <ul className="max-h-48 space-y-2 overflow-y-auto text-sm">
            {profiles.map((profile) => (
              <li key={profile.id} className="break-words">
                {profile.name}
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-xs text-muted-foreground">
            {t(
              profiles
                ? "appFeedback.noAssignments"
                : failed
                  ? "appFeedback.referencesFailed"
                  : "common.buttons.loading",
            )}
          </p>
        )}
      </PopoverContent>
    </Popover>
  );
}
