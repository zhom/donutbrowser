"use client";

import { useTranslation } from "react-i18next";
import { OperationFlow } from "@/components/ui/operation-flow";
import { useLaunchActivity } from "@/hooks/use-launch-activity";
import { translateBackendError } from "@/lib/backend-errors";

export function ProfileLaunchActivity({
  profileId,
  running,
}: {
  profileId: string;
  running: boolean;
}) {
  const { t } = useTranslation();
  const events = useLaunchActivity(profileId);
  const current = events.at(-1);
  if (!current) return null;
  const terminal = current.stage === "running" || current.stage === "failed";
  const lastStage = events.findLast((event) => event.stage !== "failed")?.stage;
  const active = ["network", "extensions"].includes(lastStage ?? "")
    ? 1
    : ["starting", "running"].includes(lastStage ?? "")
      ? 2
      : 0;
  const observedAt = (stage: string) => {
    const event = events.find((entry) => entry.stage === stage);
    return event
      ? new Date(event.timestamp).toLocaleTimeString()
      : t("appFeedback.notObserved");
  };
  return (
    <div
      data-slot="profile-launch-activity"
      data-stage={current.stage}
      className="space-y-2 rounded-md bg-muted/30 p-3"
    >
      <p role="status" className="text-xs font-medium">
        {t(
          current.stage === "running" && !running
            ? "appFeedback.launch.stopped"
            : `appFeedback.launch.${current.stage}`,
        )}
      </p>
      <OperationFlow
        label={t("appFeedback.launchActivity")}
        active={active}
        busy={!terminal}
        failed={current.stage === "failed"}
        steps={[
          {
            id: "prepare",
            label: t("appFeedback.launch.preparing"),
            detail: observedAt("preparing"),
          },
          {
            id: "network",
            label: t("profileInfo.sections.network"),
            detail: observedAt("network"),
          },
          {
            id: "browser",
            label: t("appFeedback.browser"),
            detail: terminal
              ? t(
                  current.stage === "failed"
                    ? "appFeedback.launch.failed"
                    : "appFeedback.finished",
                )
              : t("appFeedback.waiting"),
          },
        ]}
      />
      <details>
        <summary className="cursor-pointer rounded-sm text-xs text-muted-foreground focus-visible:outline-2 focus-visible:outline-ring">
          {t("appFeedback.launchActivity")}
        </summary>
        <ol className="mt-2 space-y-2 text-xs">
          {events.map((event, index) => (
            <li
              key={`${index}:${event.timestamp}`}
              className="flex items-start gap-3"
            >
              <time
                className="shrink-0 tabular-nums text-muted-foreground"
                dateTime={new Date(event.timestamp).toISOString()}
              >
                {new Date(event.timestamp).toLocaleTimeString()}
              </time>
              <span>{t(`appFeedback.launch.${event.stage}`)}</span>
            </li>
          ))}
        </ol>
      </details>
      {current.error && (
        <p className="break-words text-xs text-destructive-text">
          {translateBackendError(t, current.error)}
        </p>
      )}
    </div>
  );
}
