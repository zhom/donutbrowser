"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { LuPlus, LuTrash2 } from "react-icons/lu";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type { BrowserProfile, ProfileStatus } from "@/types";

/**
 * Data colors for statuses, picked to stay readable on light and dark themes.
 * They label accounts the person tracks; they are not theme colors.
 */
export const STATUS_COLORS = [
  "#22c55e",
  "#84cc16",
  "#eab308",
  "#f97316",
  "#ef4444",
  "#ec4899",
  "#a855f7",
  "#3b82f6",
  "#06b6d4",
  "#94a3b8",
] as const;

/** The set offered when there are no statuses yet. */
const STARTER_STATUSES = [
  { key: "profileStatuses.starter.active", color: "#22c55e" },
  { key: "profileStatuses.starter.warming", color: "#eab308" },
  { key: "profileStatuses.starter.review", color: "#3b82f6" },
  { key: "profileStatuses.starter.banned", color: "#ef4444" },
] as const;

export function ProfileStatusDot({
  color,
  className,
}: {
  color: string;
  className?: string;
}) {
  return (
    <span
      aria-hidden
      className={cn("inline-block size-2 shrink-0 rounded-full", className)}
      style={{ backgroundColor: color }}
    />
  );
}

export function ProfileStatusChip({ status }: { status: ProfileStatus }) {
  return (
    <span
      className="flex min-w-0 items-center gap-1.5"
      data-slot="profile-status-chip"
    >
      <ProfileStatusDot color={status.color} />
      <span className="truncate">{status.name}</span>
    </span>
  );
}

function ColorChoice({
  value,
  onChange,
  disabled,
}: {
  value: string;
  onChange: (color: string) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          aria-label={t("profileStatuses.color")}
          className="grid size-8 shrink-0 place-items-center rounded-md transition-colors duration-100 hover:bg-foreground/6 disabled:opacity-50"
        >
          <ProfileStatusDot color={value} className="size-3.5" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-auto p-2">
        <div className="grid grid-cols-5 gap-1">
          {STATUS_COLORS.map((color) => (
            <button
              key={color}
              type="button"
              aria-label={color}
              aria-pressed={value === color}
              onClick={() => onChange(color)}
              className={cn(
                "grid size-7 place-items-center rounded-md transition-colors duration-100 hover:bg-foreground/6",
                value === color && "bg-foreground/10",
              )}
            >
              <ProfileStatusDot color={color} className="size-3.5" />
            </button>
          ))}
        </div>
        <label className="mt-2 flex cursor-pointer items-center gap-2 rounded-md px-1 py-1 text-xs text-muted-foreground hover:bg-foreground/6">
          <input
            type="color"
            value={value}
            onChange={(event) => onChange(event.target.value)}
            className="size-5 cursor-pointer rounded border-0 bg-transparent p-0"
          />
          {t("profileStatuses.customColor")}
        </label>
      </PopoverContent>
    </Popover>
  );
}

function StatusRow({
  status,
  usage,
  onDelete,
}: {
  status: ProfileStatus;
  usage: number;
  onDelete: (status: ProfileStatus) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = React.useState(status.name);

  React.useEffect(() => {
    setName(status.name);
  }, [status.name]);

  const save = async (next: { name?: string; color?: string }) => {
    const nextName = (next.name ?? name).trim();
    const nextColor = next.color ?? status.color;
    if (nextName === status.name && nextColor === status.color) return;
    try {
      await invoke("update_profile_status", {
        statusId: status.id,
        name: nextName,
        color: nextColor,
      });
    } catch (error) {
      setName(status.name);
      showErrorToast(translateBackendError(t, error));
    }
  };

  return (
    <li className="flex items-center gap-1" data-slot="profile-status-row">
      <ColorChoice
        value={status.color}
        onChange={(color) => void save({ color })}
      />
      <Input
        variant="soft"
        value={name}
        aria-label={t("profileStatuses.namePlaceholder")}
        onChange={(event) => setName(event.target.value)}
        onBlur={() => void save({})}
        onKeyDown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
          if (event.key === "Escape") setName(status.name);
        }}
        className="h-8 flex-1"
      />
      <span className="w-12 shrink-0 text-right text-xs text-muted-foreground tabular-nums">
        {usage}
      </span>
      <Button
        type="button"
        variant="subtle"
        size="icon"
        className="size-8"
        aria-label={t("profileStatuses.delete")}
        onClick={() => onDelete(status)}
      >
        <LuTrash2 className="size-3.5" />
      </Button>
    </li>
  );
}

export function ProfileStatusesDialog({
  isOpen,
  onClose,
  statuses,
  profiles,
}: {
  isOpen: boolean;
  onClose: () => void;
  statuses: ProfileStatus[];
  profiles: BrowserProfile[];
}) {
  const { t } = useTranslation();
  const [newName, setNewName] = React.useState("");
  const [newColor, setNewColor] = React.useState<string>(STATUS_COLORS[0]);
  const [isAdding, setIsAdding] = React.useState(false);
  const [deleting, setDeleting] = React.useState<ProfileStatus | null>(null);

  const usage = React.useMemo(() => {
    const counts = new Map<string, number>();
    for (const profile of profiles) {
      if (profile.status_id) {
        counts.set(profile.status_id, (counts.get(profile.status_id) ?? 0) + 1);
      }
    }
    return counts;
  }, [profiles]);

  React.useEffect(() => {
    if (isOpen) {
      setNewName("");
      setNewColor(STATUS_COLORS[statuses.length % STATUS_COLORS.length]);
    }
  }, [isOpen, statuses.length]);

  const add = async (entries: { name: string; color: string }[]) => {
    setIsAdding(true);
    try {
      for (const entry of entries) {
        await invoke("create_profile_status", entry);
      }
      setNewName("");
    } catch (error) {
      showErrorToast(translateBackendError(t, error));
    } finally {
      setIsAdding(false);
    }
  };

  const confirmDelete = async () => {
    if (!deleting) return;
    try {
      await invoke("delete_profile_status", { statusId: deleting.id });
    } catch (error) {
      showErrorToast(translateBackendError(t, error));
    } finally {
      setDeleting(null);
    }
  };

  return (
    <>
      <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
        <DialogContent className="max-w-md" data-slot="profile-statuses-dialog">
          <DialogHeader>
            <DialogTitle>{t("profileStatuses.title")}</DialogTitle>
            <DialogDescription>
              {t("profileStatuses.description")}
            </DialogDescription>
          </DialogHeader>

          {statuses.length === 0 ? (
            <div className="flex flex-col items-start gap-2 rounded-lg bg-foreground/4 p-3">
              <p className="text-sm text-muted-foreground">
                {t("profileStatuses.empty")}
              </p>
              <Button
                type="button"
                variant="soft"
                size="sm"
                disabled={isAdding}
                onClick={() =>
                  void add(
                    STARTER_STATUSES.map((entry) => ({
                      name: t(entry.key),
                      color: entry.color,
                    })),
                  )
                }
              >
                {t("profileStatuses.addStarter")}
              </Button>
            </div>
          ) : (
            <ul className="flex max-h-80 flex-col gap-1 overflow-y-auto">
              {statuses.map((status) => (
                <StatusRow
                  key={status.id}
                  status={status}
                  usage={usage.get(status.id) ?? 0}
                  onDelete={setDeleting}
                />
              ))}
            </ul>
          )}

          <form
            className="flex items-center gap-1"
            onSubmit={(event) => {
              event.preventDefault();
              if (newName.trim()) {
                void add([{ name: newName.trim(), color: newColor }]);
              }
            }}
          >
            <ColorChoice
              value={newColor}
              onChange={setNewColor}
              disabled={isAdding}
            />
            <Input
              id="new-profile-status"
              variant="soft"
              value={newName}
              placeholder={t("profileStatuses.namePlaceholder")}
              onChange={(event) => setNewName(event.target.value)}
              disabled={isAdding}
              className="h-8 flex-1"
            />
            <Button
              type="submit"
              size="sm"
              className="h-8"
              disabled={isAdding || !newName.trim()}
            >
              <LuPlus className="size-3.5" />
              {t("common.buttons.add")}
            </Button>
          </form>
        </DialogContent>
      </Dialog>
      <DeleteConfirmationDialog
        isOpen={deleting !== null}
        onClose={() => setDeleting(null)}
        onConfirm={confirmDelete}
        title={t("profileStatuses.deleteTitle", {
          name: deleting?.name ?? "",
        })}
        description={t("profileStatuses.deleteDescription")}
        confirmButtonText={t("common.buttons.delete")}
      />
    </>
  );
}
