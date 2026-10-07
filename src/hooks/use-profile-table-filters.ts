"use client";

import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useTableFilters } from "@/hooks/use-table-tools";
import {
  matchesProfile,
  type ProfileSearchContext,
  parseProfileSearch,
} from "@/lib/profile-search";
import type { TableFilter } from "@/lib/table-tools";
import type { BrowserProfile } from "@/types";

const noTextSearch = () => "";

export function useProfileTableFilters(
  profiles: BrowserProfile[],
  query: string,
  context: ProfileSearchContext,
  clearSelection: () => void,
) {
  const { t } = useTranslation();
  const searched = useMemo(() => {
    const parsed = parseProfileSearch(query);
    return parsed.isEmpty
      ? profiles
      : profiles.filter((profile) => matchesProfile(profile, parsed, context));
  }, [profiles, query, context]);
  const definitions = useMemo<TableFilter<BrowserProfile>[]>(
    () => [
      {
        id: "status",
        label: t("search.fields.status"),
        value: (row) =>
          context.runningProfiles.has(row.id) ? "running" : "stopped",
        options: [
          { value: "running", label: t("tables.running") },
          { value: "stopped", label: t("tables.stopped") },
        ],
      },
      {
        id: "tag",
        label: t("profileTable.tagsHeader"),
        value: (row) => (row.tags?.length ? row.tags : [""]),
        options: [
          { value: "", label: t("tables.noTags") },
          ...[...new Set(profiles.flatMap((row) => row.tags ?? []))]
            .sort()
            .map((value) => ({ value, label: value })),
        ],
      },
      {
        id: "proxy",
        label: t("profiles.table.proxy"),
        value: (row) => row.proxy_id ?? "",
        options: [
          { value: "", label: t("tables.noProxy") },
          ...[...context.proxyNames].map(([value, label]) => ({
            value,
            label,
          })),
        ],
      },
      {
        id: "sync",
        label: t("proxies.management.syncCol"),
        value: (row) => row.sync_mode ?? "Disabled",
        options: [
          { value: "Disabled", label: t("tables.disabled") },
          { value: "Regular", label: t("tables.enabled") },
          { value: "Encrypted", label: t("tables.encrypted") },
        ],
      },
    ],
    [profiles, context, t],
  );
  return useTableFilters(searched, noTextSearch, definitions, clearSelection);
}
