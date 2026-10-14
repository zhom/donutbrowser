"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { Button } from "@/components/ui/button";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";

interface RestorePoint {
  id: string;
  createdAt: string;
  kind: "auto" | "manual";
  files: number;
  bytes: number;
}

function formatBytes(bytes: number, language: string): string {
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toLocaleString(language, { maximumFractionDigits: 1 })} ${units[unit]}`;
}

/** The copies the sync server keeps of a synced profile's data. */
export function RestorePoints({
  profileId,
  isDisabled,
}: {
  profileId: string;
  isDisabled: boolean;
}) {
  const { t, i18n } = useTranslation();
  const [points, setPoints] = React.useState<RestorePoint[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);
  const [restoring, setRestoring] = React.useState<RestorePoint | null>(null);

  const load = React.useCallback(async () => {
    try {
      setPoints(
        await invoke<RestorePoint[]>("list_restore_points", { profileId }),
      );
      setError(null);
    } catch (err) {
      setPoints(null);
      setError(translateBackendError(t, err));
    }
  }, [profileId, t]);

  React.useEffect(() => {
    void load();
  }, [load]);

  const run = async (work: () => Promise<unknown>, done: string) => {
    setBusy(true);
    try {
      await work();
      showSuccessToast(done);
      await load();
    } catch (err) {
      showErrorToast(translateBackendError(t, err));
    } finally {
      setBusy(false);
    }
  };

  const when = (iso: string) => {
    const date = new Date(iso);
    return Number.isNaN(date.getTime())
      ? iso
      : date.toLocaleString(i18n.language, {
          dateStyle: "medium",
          timeStyle: "short",
        });
  };

  return (
    <div
      className="rounded-lg bg-foreground/4 px-3 py-2"
      data-slot="restore-points"
    >
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs font-medium text-muted-foreground">
          {t("restorePoints.title")}
        </p>
        <Button
          variant="soft"
          size="sm"
          className="h-7"
          disabled={isDisabled || busy || points === null}
          onClick={() =>
            void run(
              () => invoke("create_restore_point", { profileId }),
              t("restorePoints.created"),
            )
          }
          data-slot="restore-point-create"
        >
          {t("restorePoints.create")}
        </Button>
      </div>
      <p className="mt-0.5 text-xs text-muted-foreground">
        {t("restorePoints.hint")}
      </p>
      {error && <p className="mt-1 text-xs text-destructive-text">{error}</p>}
      {points && points.length === 0 && (
        <p className="mt-1 text-xs text-muted-foreground">
          {t("restorePoints.empty")}
        </p>
      )}
      {points && points.length > 0 && (
        <ul className="mt-1 flex flex-col">
          {points.map((point) => (
            <li
              key={point.id}
              className="row-reveal flex items-center justify-between gap-2 rounded-md px-1 py-1 text-sm hover:bg-foreground/5"
              data-point={point.id}
            >
              <span className="min-w-0 truncate">
                {when(point.createdAt)}{" "}
                <span className="text-xs text-muted-foreground">
                  {t(`restorePoints.kind.${point.kind}`)} ·{" "}
                  {formatBytes(point.bytes, i18n.language)}
                </span>
              </span>
              <span className="flex shrink-0 gap-1">
                <Button
                  variant="subtle"
                  size="sm"
                  className="h-7"
                  disabled={isDisabled || busy}
                  onClick={() => setRestoring(point)}
                  data-slot="restore-point-restore"
                >
                  {t("restorePoints.restore")}
                </Button>
                <Button
                  variant="subtle"
                  size="sm"
                  className="h-7"
                  disabled={isDisabled || busy}
                  onClick={() =>
                    void run(
                      () =>
                        invoke("delete_restore_point", {
                          profileId,
                          id: point.id,
                        }),
                      t("restorePoints.deleted"),
                    )
                  }
                >
                  {t("common.buttons.delete")}
                </Button>
              </span>
            </li>
          ))}
        </ul>
      )}
      <DeleteConfirmationDialog
        isOpen={restoring !== null}
        onClose={() => setRestoring(null)}
        onConfirm={async () => {
          const point = restoring;
          setRestoring(null);
          if (!point) return;
          await run(
            () => invoke("restore_restore_point", { profileId, id: point.id }),
            t("restorePoints.restored"),
          );
        }}
        title={t("restorePoints.restoreTitle")}
        description={t("restorePoints.restoreDescription", {
          when: restoring ? when(restoring.createdAt) : "",
        })}
        confirmButtonText={t("restorePoints.restore")}
        isLoading={busy}
      />
    </div>
  );
}
