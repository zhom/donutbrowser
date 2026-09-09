"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import { LuChevronDown, LuChevronUp, LuTrash2 } from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { translateBackendError } from "@/lib/backend-errors";
import { showSuccessToast } from "@/lib/toast-utils";
import type { BrowserProfile, GroupBookmark } from "@/types";
import { RippleButton } from "./ui/ripple";

interface GroupBookmarksDialogProps {
  isOpen: boolean;
  onClose: () => void;
  groupId: string | null;
  groupName: string;
  onBookmarksSaved: () => void;
}

/** A row carries its own key so React keeps focus while a row is reordered. */
interface BookmarkRow extends GroupBookmark {
  key: string;
}

function newKey(): string {
  return `bookmark-${Math.random().toString(36).slice(2)}`;
}

function toRows(bookmarks: GroupBookmark[]): BookmarkRow[] {
  return bookmarks.map((bookmark) => ({
    key: newKey(),
    title: bookmark.title,
    url: bookmark.url,
    folder: bookmark.folder ?? "",
  }));
}

export function GroupBookmarksDialog({
  isOpen,
  onClose,
  groupId,
  groupName,
  onBookmarksSaved,
}: GroupBookmarksDialogProps) {
  const { t } = useTranslation();
  const [rows, setRows] = useState<BookmarkRow[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isOpen || !groupId) return;
    setError(null);
    setIsLoading(true);
    void invoke<GroupBookmark[]>("get_group_bookmarks", { groupId })
      .then((bookmarks) => {
        setRows(toRows(bookmarks));
      })
      .catch((err: unknown) => {
        console.error("Failed to load group bookmarks:", err);
        setError(translateBackendError(t, err));
      })
      .finally(() => {
        setIsLoading(false);
      });
  }, [isOpen, groupId, t]);

  const updateRow = useCallback(
    (key: string, field: keyof GroupBookmark, value: string) => {
      setRows((current) =>
        current.map((row) =>
          row.key === key ? { ...row, [field]: value } : row,
        ),
      );
    },
    [],
  );

  const moveRow = useCallback((index: number, delta: number) => {
    setRows((current) => {
      const target = index + delta;
      if (target < 0 || target >= current.length) return current;
      const next = [...current];
      const [moved] = next.splice(index, 1);
      next.splice(target, 0, moved);
      return next;
    });
  }, []);

  const handleSave = useCallback(async () => {
    if (!groupId) return;
    setIsSaving(true);
    setError(null);
    try {
      const bookmarks: GroupBookmark[] = rows.map((row) => ({
        title: row.title,
        url: row.url,
        folder: row.folder?.trim() ? row.folder.trim() : null,
      }));
      const saved = await invoke<GroupBookmark[]>("set_group_bookmarks", {
        groupId,
        bookmarks,
      });
      setRows(toRows(saved));

      // Push the edit to the group's stopped profiles now instead of making
      // everyone wait for their next launch. Best effort on purpose: a profile
      // that refuses the write (it just started, its file is unreadable) picks
      // the change up on launch anyway, and the save itself already succeeded.
      const members = await invoke<BrowserProfile[]>("list_browser_profiles");
      await Promise.allSettled(
        members
          .filter((profile) => profile.group_id === groupId)
          .map((profile) =>
            invoke("apply_group_bookmarks_to_profile", {
              profileId: profile.id,
            }),
          ),
      );

      showSuccessToast(t("groupBookmarks.saved"));
      onBookmarksSaved();
      onClose();
    } catch (err) {
      console.error("Failed to save group bookmarks:", err);
      setError(translateBackendError(t, err));
    } finally {
      setIsSaving(false);
    }
  }, [groupId, rows, onBookmarksSaved, onClose, t]);

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="flex max-h-[85vh] max-w-2xl flex-col">
        <DialogHeader>
          <DialogTitle>
            {t("groupBookmarks.title", { name: groupName })}
          </DialogTitle>
          <DialogDescription>
            {t("groupBookmarks.description")}
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto">
          {isLoading ? (
            <p className="text-sm text-muted-foreground">
              {t("common.buttons.loading")}
            </p>
          ) : rows.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {t("groupBookmarks.empty")}
            </p>
          ) : (
            <div className="space-y-3" data-testid="group-bookmark-rows">
              {rows.map((row, index) => (
                <div
                  key={row.key}
                  data-testid="group-bookmark-row"
                  className="flex items-end gap-2 rounded-md border border-border p-3"
                >
                  <div className="grid min-w-0 flex-1 gap-2 sm:grid-cols-3">
                    <div className="space-y-1">
                      <Label htmlFor={`${row.key}-title`} className="text-xs">
                        {t("groupBookmarks.titleColumn")}
                      </Label>
                      <Input
                        id={`${row.key}-title`}
                        data-testid="group-bookmark-title"
                        value={row.title}
                        placeholder={t("groupBookmarks.titlePlaceholder")}
                        onChange={(event) => {
                          updateRow(row.key, "title", event.target.value);
                        }}
                        disabled={isSaving}
                      />
                    </div>
                    <div className="space-y-1">
                      <Label htmlFor={`${row.key}-url`} className="text-xs">
                        {t("groupBookmarks.urlColumn")}
                      </Label>
                      <Input
                        id={`${row.key}-url`}
                        data-testid="group-bookmark-url"
                        value={row.url}
                        placeholder={t("groupBookmarks.urlPlaceholder")}
                        onChange={(event) => {
                          updateRow(row.key, "url", event.target.value);
                        }}
                        disabled={isSaving}
                      />
                    </div>
                    <div className="space-y-1">
                      <Label htmlFor={`${row.key}-folder`} className="text-xs">
                        {t("groupBookmarks.folderColumn")}
                      </Label>
                      <Input
                        id={`${row.key}-folder`}
                        data-testid="group-bookmark-folder"
                        value={row.folder ?? ""}
                        placeholder={t("groupBookmarks.folderPlaceholder")}
                        onChange={(event) => {
                          updateRow(row.key, "folder", event.target.value);
                        }}
                        disabled={isSaving}
                      />
                    </div>
                  </div>
                  <div className="flex shrink-0 gap-1">
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          variant="ghost"
                          size="sm"
                          data-testid="group-bookmark-move-up"
                          aria-label={t("groupBookmarks.moveUp")}
                          disabled={index === 0 || isSaving}
                          onClick={() => {
                            moveRow(index, -1);
                          }}
                        >
                          <LuChevronUp className="size-4" />
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>
                        <p>{t("groupBookmarks.moveUp")}</p>
                      </TooltipContent>
                    </Tooltip>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          variant="ghost"
                          size="sm"
                          data-testid="group-bookmark-move-down"
                          aria-label={t("groupBookmarks.moveDown")}
                          disabled={index === rows.length - 1 || isSaving}
                          onClick={() => {
                            moveRow(index, 1);
                          }}
                        >
                          <LuChevronDown className="size-4" />
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>
                        <p>{t("groupBookmarks.moveDown")}</p>
                      </TooltipContent>
                    </Tooltip>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          variant="ghost"
                          size="sm"
                          data-testid="group-bookmark-remove"
                          aria-label={t("groupBookmarks.removeRow")}
                          disabled={isSaving}
                          onClick={() => {
                            setRows((current) =>
                              current.filter((item) => item.key !== row.key),
                            );
                          }}
                        >
                          <LuTrash2 className="size-4" />
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>
                        <p>{t("groupBookmarks.removeRow")}</p>
                      </TooltipContent>
                    </Tooltip>
                  </div>
                </div>
              ))}
            </div>
          )}

          <RippleButton
            variant="outline"
            size="sm"
            data-testid="group-bookmark-add"
            disabled={isSaving}
            onClick={() => {
              setRows((current) => [
                ...current,
                { key: newKey(), title: "", url: "", folder: "" },
              ]);
            }}
            className="flex items-center gap-2"
          >
            <GoPlus className="size-4" />
            {t("groupBookmarks.addRow")}
          </RippleButton>

          {error && (
            <div className="rounded-md bg-destructive/10 p-3 text-sm text-destructive-text">
              {error}
            </div>
          )}
        </div>

        <DialogFooter>
          <RippleButton variant="outline" onClick={onClose} disabled={isSaving}>
            {t("common.buttons.cancel")}
          </RippleButton>
          <LoadingButton
            isLoading={isSaving}
            data-testid="group-bookmark-save"
            onClick={() => void handleSave()}
          >
            {t("common.buttons.save")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
