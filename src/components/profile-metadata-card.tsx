"use client";

import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import MultipleSelector, { type Option } from "@/components/multiple-selector";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Textarea } from "@/components/ui/textarea";
import { translateBackendError } from "@/lib/backend-errors";
import type { BrowserProfile } from "@/types";

export function ProfileMetadataCard({
  profile,
  field,
  disabled,
}: {
  profile: BrowserProfile;
  field: "tags" | "note";
  disabled: boolean;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState(profile.note ?? "");
  const [tags, setTags] = useState<Option[]>(
    (profile.tags ?? []).map((value) => ({ value, label: value })),
  );
  const [options, setOptions] = useState<Option[]>(tags);
  useEffect(() => {
    if (!open || field !== "tags") return;
    let active = true;
    void invoke<string[]>("get_all_tags")
      .then((values) => {
        if (active)
          setOptions(values.map((value) => ({ value, label: value })));
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [open, field]);
  const save = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      if (field === "tags")
        await invoke("update_profile_tags", {
          profileId: profile.id,
          tags: [...new Set(tags.map((tag) => tag.value))],
        });
      else
        await invoke("update_profile_note", {
          profileId: profile.id,
          note: note.trim() || null,
        });
      setOpen(false);
    } catch (err) {
      setError(translateBackendError(t, err));
    } finally {
      setBusy(false);
    }
  };
  const value =
    field === "tags" ? (profile.tags ?? []).join(", ") : profile.note;
  const label = t(`profileInfo.fields.${field}`);
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        if (busy) return;
        if (next) {
          setNote(profile.note ?? "");
          setTags(
            (profile.tags ?? []).map((tag) => ({ value: tag, label: tag })),
          );
          setError(null);
        }
        setOpen(next);
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          data-slot={`profile-edit-${field}`}
          className="min-w-0 rounded-md bg-muted/50 px-3 py-2.5 text-left transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-50"
        >
          <span className="block text-xs text-muted-foreground">{label}</span>
          <span className="mt-0.5 block break-words text-sm">
            {value || t("profileInfo.values.none")}
          </span>
        </button>
      </PopoverTrigger>
      <PopoverContent
        className="w-[min(24rem,calc(100vw-2rem))] space-y-3 p-3"
        align="start"
      >
        <h3 className="text-sm font-medium">{label}</h3>
        {field === "tags" ? (
          <MultipleSelector
            value={tags}
            options={options}
            creatable
            onChange={setTags}
            disabled={busy}
            inputProps={{ "aria-label": label }}
            placeholder={t("profileTable.addTagsPlaceholder")}
          />
        ) : (
          <Textarea
            aria-label={label}
            value={note}
            disabled={busy}
            onChange={(event) => setNote(event.target.value)}
            className="max-h-60 min-h-28"
          />
        )}
        {error && (
          <p role="alert" className="break-words text-xs text-destructive-text">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <Button
            size="sm"
            variant="ghost"
            disabled={busy}
            onClick={() => setOpen(false)}
          >
            {t("common.buttons.cancel")}
          </Button>
          <Button size="sm" disabled={busy} onClick={() => void save()}>
            {t(busy ? "common.buttons.saving" : "common.buttons.save")}
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
