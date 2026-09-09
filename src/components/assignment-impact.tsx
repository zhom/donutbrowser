"use client";

import { useTranslation } from "react-i18next";
import { OperationFlow } from "@/components/ui/operation-flow";
import type { BrowserProfile, ExtensionGroup } from "@/types";

export function AssignmentImpact({
  sources,
  groups,
  profiles,
  failed,
  pending = false,
}: {
  sources: string[];
  groups: ExtensionGroup[];
  profiles: BrowserProfile[] | null;
  failed: boolean;
  pending?: boolean;
}) {
  const { t } = useTranslation();
  const assigned = profiles?.filter((profile) =>
    groups.some((group) => group.id === profile.extension_group_id),
  );
  const names = (label: string, values: string[]) => (
    <details className="group">
      <summary className="cursor-pointer rounded-sm px-1 py-1 focus-visible:outline-2 focus-visible:outline-ring">
        {t(label, { count: values.length })}
      </summary>
      <ul className="mt-1 max-h-36 space-y-1 overflow-y-auto text-left">
        {values.map((name, index) => (
          <li key={`${index}:${name}`} className="break-words">
            {name}
          </li>
        ))}
      </ul>
    </details>
  );
  return (
    <div
      data-slot="assignment-impact"
      className="space-y-2 rounded-md bg-muted/30 p-3"
    >
      <p className="text-xs font-medium">
        {t(
          pending
            ? "appFeedback.pendingImpact"
            : "appFeedback.assignmentImpact",
        )}
      </p>
      <OperationFlow
        label={t("appFeedback.assignmentImpact")}
        active={pending ? 0 : 2}
        steps={[
          {
            id: "extensions",
            label: t("extensions.extensionsTab"),
            detail: names("appFeedback.extensionCount", sources),
          },
          {
            id: "groups",
            label: t("extensions.groupsTab"),
            detail: names(
              "appFeedback.groupCount",
              groups.map((group) => group.name),
            ),
          },
          {
            id: "profiles",
            label: t("profiles.title"),
            detail: assigned
              ? names(
                  "appFeedback.profileCount",
                  assigned.map((profile) => profile.name),
                )
              : t(
                  failed
                    ? "appFeedback.referencesFailed"
                    : "common.buttons.loading",
                ),
          },
        ]}
      />
      <p className="text-xs leading-relaxed text-muted-foreground">
        {t("appFeedback.nextLaunchImpact")}
      </p>
    </div>
  );
}
