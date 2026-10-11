"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { DnsServerField } from "@/components/dns-server-field";
import { ProfileStatusChip } from "@/components/profile-statuses-dialog";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { translateBackendError } from "@/lib/backend-errors";
import { DNS_BLOCKLIST_LEVELS } from "@/lib/dns-blocklist-levels";
import { startPagesFromText } from "@/lib/start-pages";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type { ProfileStatus } from "@/types";

type Field =
  | "status"
  | "tagsAdd"
  | "tagsRemove"
  | "note"
  | "dnsBlocklist"
  | "dnsServer"
  | "startPages"
  | "clearOnClose"
  | "webrtc"
  | "blockImages"
  | "blockWebgl"
  | "randomize"
  | "restoreSession"
  | "newDevice";

const NONE = "__none__";

function tagsFrom(text: string): string[] {
  return text
    .split(",")
    .map((tag) => tag.trim())
    .filter(Boolean);
}

function Row({
  field,
  label,
  enabled,
  onToggle,
  children,
}: {
  field: Field;
  label: string;
  enabled: boolean;
  onToggle: (field: Field, on: boolean) => void;
  children?: React.ReactNode;
}) {
  const id = `bulk-edit-${field}`;
  return (
    <div
      className="flex items-start gap-3 py-1.5"
      data-slot="bulk-edit-row"
      data-field={field}
    >
      <Checkbox
        id={id}
        checked={enabled}
        onCheckedChange={(checked) => onToggle(field, checked === true)}
        className="mt-0.5"
      />
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <label
          htmlFor={id}
          className={cn(
            "text-sm",
            enabled ? "text-foreground" : "text-muted-foreground",
          )}
        >
          {label}
        </label>
        {enabled && children}
      </div>
    </div>
  );
}

function OnOff({
  value,
  onChange,
}: {
  value: boolean;
  onChange: (value: boolean) => void;
}) {
  return <AnimatedSwitch checked={value} onCheckedChange={onChange} />;
}

export function BulkEditDialog({
  isOpen,
  onClose,
  profileIds,
  statuses,
}: {
  isOpen: boolean;
  onClose: () => void;
  profileIds: string[];
  statuses: ProfileStatus[];
}) {
  const { t } = useTranslation();
  const [enabled, setEnabled] = React.useState<Set<Field>>(new Set());
  const [statusId, setStatusId] = React.useState<string>(NONE);
  const [tagsAdd, setTagsAdd] = React.useState("");
  const [tagsRemove, setTagsRemove] = React.useState("");
  const [note, setNote] = React.useState("");
  const [dnsBlocklist, setDnsBlocklist] = React.useState<string>(NONE);
  const [dnsServer, setDnsServer] = React.useState("");
  const [dnsServerValid, setDnsServerValid] = React.useState(true);
  const [startPages, setStartPages] = React.useState("");
  const [clearOnClose, setClearOnClose] = React.useState(false);
  const [webrtc, setWebrtc] = React.useState("auto");
  const [blockImages, setBlockImages] = React.useState(false);
  const [blockWebgl, setBlockWebgl] = React.useState(false);
  const [randomize, setRandomize] = React.useState(false);
  const [restoreSession, setRestoreSession] = React.useState(true);
  const [isSaving, setIsSaving] = React.useState(false);

  React.useEffect(() => {
    if (!isOpen) return;
    setEnabled(new Set());
    setStatusId(NONE);
    setTagsAdd("");
    setTagsRemove("");
    setNote("");
    setDnsBlocklist(NONE);
    setDnsServer("");
    setStartPages("");
    setClearOnClose(false);
    setWebrtc("auto");
    setBlockImages(false);
    setBlockWebgl(false);
    setRandomize(false);
    setRestoreSession(true);
  }, [isOpen]);

  const toggle = (field: Field, on: boolean) => {
    setEnabled((current) => {
      const next = new Set(current);
      if (on) next.add(field);
      else next.delete(field);
      return next;
    });
  };
  const has = (field: Field) => enabled.has(field);

  const buildPatch = () => {
    const patch: Record<string, unknown> = {};
    if (has("status")) patch.status_id = statusId === NONE ? null : statusId;
    if (has("tagsAdd")) patch.tags_add = tagsFrom(tagsAdd);
    if (has("tagsRemove")) patch.tags_remove = tagsFrom(tagsRemove);
    if (has("note")) patch.note = note.trim() || null;
    if (has("dnsBlocklist"))
      patch.dns_blocklist = dnsBlocklist === NONE ? null : dnsBlocklist;
    if (has("dnsServer")) patch.dns_server = dnsServer.trim() || null;
    if (has("startPages")) patch.start_urls = startPagesFromText(startPages);
    if (has("clearOnClose")) patch.clear_on_close = clearOnClose;
    if (has("webrtc")) patch.webrtc_mode = webrtc;
    if (has("blockImages")) patch.block_images = blockImages;
    if (has("blockWebgl")) patch.block_webgl = blockWebgl;
    if (has("randomize")) patch.randomize_fingerprint_on_launch = randomize;
    if (has("restoreSession")) patch.restore_session = restoreSession;
    if (has("newDevice")) patch.new_device = true;
    return patch;
  };

  const apply = async () => {
    setIsSaving(true);
    try {
      const changed = await invoke<number>("bulk_update_profiles", {
        profileIds,
        patch: buildPatch(),
      });
      showSuccessToast(t("bulkEdit.applied", { count: changed }));
      onClose();
    } catch (error) {
      showErrorToast(translateBackendError(t, error));
    } finally {
      setIsSaving(false);
    }
  };

  const blocked = has("dnsServer") && !dnsServerValid;

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        className="flex max-h-[85vh] max-w-lg flex-col"
        data-slot="bulk-edit-dialog"
      >
        <DialogHeader>
          <DialogTitle>{t("bulkEdit.title")}</DialogTitle>
          <DialogDescription>{t("bulkEdit.description")}</DialogDescription>
        </DialogHeader>
        <div className="-mx-1 min-h-0 flex-1 divide-y divide-border/50 overflow-y-auto px-1">
          <Row
            field="status"
            label={t("profileStatuses.column")}
            enabled={has("status")}
            onToggle={toggle}
          >
            <Select value={statusId} onValueChange={setStatusId}>
              <SelectTrigger variant="soft" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NONE}>
                  {t("profileStatuses.none")}
                </SelectItem>
                {statuses.map((status) => (
                  <SelectItem key={status.id} value={status.id}>
                    <ProfileStatusChip status={status} />
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Row>
          <Row
            field="tagsAdd"
            label={t("bulkEdit.tagsAdd")}
            enabled={has("tagsAdd")}
            onToggle={toggle}
          >
            <Input
              variant="soft"
              value={tagsAdd}
              onChange={(event) => setTagsAdd(event.target.value)}
              placeholder={t("bulkEdit.tagsPlaceholder")}
            />
          </Row>
          <Row
            field="tagsRemove"
            label={t("bulkEdit.tagsRemove")}
            enabled={has("tagsRemove")}
            onToggle={toggle}
          >
            <Input
              variant="soft"
              value={tagsRemove}
              onChange={(event) => setTagsRemove(event.target.value)}
              placeholder={t("bulkEdit.tagsPlaceholder")}
            />
          </Row>
          <Row
            field="note"
            label={t("profileTable.noteHeader")}
            enabled={has("note")}
            onToggle={toggle}
          >
            <Textarea
              variant="soft"
              value={note}
              onChange={(event) => setNote(event.target.value)}
              placeholder={t("bulkEdit.notePlaceholder")}
              rows={2}
            />
          </Row>
          <Row
            field="dnsBlocklist"
            label={t("dnsBlocklist.title")}
            enabled={has("dnsBlocklist")}
            onToggle={toggle}
          >
            <Select value={dnsBlocklist} onValueChange={setDnsBlocklist}>
              <SelectTrigger variant="soft" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NONE}>{t("dnsBlocklist.none")}</SelectItem>
                {DNS_BLOCKLIST_LEVELS.map((level) => (
                  <SelectItem key={level.value} value={level.value}>
                    {t(level.labelKey)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Row>
          <Row
            field="dnsServer"
            label={t("dnsServer.label")}
            enabled={has("dnsServer")}
            onToggle={toggle}
          >
            <DnsServerField
              id="bulk-edit-dns-server-value"
              value={dnsServer}
              onChange={setDnsServer}
              onValidityChange={setDnsServerValid}
            />
          </Row>
          <Row
            field="startPages"
            label={t("profileInfo.sections.startPages")}
            enabled={has("startPages")}
            onToggle={toggle}
          >
            <Textarea
              variant="soft"
              value={startPages}
              onChange={(event) => setStartPages(event.target.value)}
              placeholder={t("startPages.placeholder")}
              rows={3}
              spellCheck={false}
              className="font-mono text-xs"
            />
          </Row>
          <Row
            field="clearOnClose"
            label={t("clearOnClose.label")}
            enabled={has("clearOnClose")}
            onToggle={toggle}
          >
            <OnOff value={clearOnClose} onChange={setClearOnClose} />
          </Row>
          <Row
            field="webrtc"
            label={t("fingerprint.webrtcModeLabel")}
            enabled={has("webrtc")}
            onToggle={toggle}
          >
            <Select value={webrtc} onValueChange={setWebrtc}>
              <SelectTrigger variant="soft" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(["auto", "tcp_only", "block"] as const).map((mode) => (
                  <SelectItem key={mode} value={mode}>
                    {t(`fingerprint.webrtcMode.${mode}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Row>
          <Row
            field="blockImages"
            label={t("fingerprint.blockImages")}
            enabled={has("blockImages")}
            onToggle={toggle}
          >
            <OnOff value={blockImages} onChange={setBlockImages} />
          </Row>
          <Row
            field="blockWebgl"
            label={t("fingerprint.blockWebGL")}
            enabled={has("blockWebgl")}
            onToggle={toggle}
          >
            <OnOff value={blockWebgl} onChange={setBlockWebgl} />
          </Row>
          <Row
            field="randomize"
            label={t("fingerprint.generateRandomOnLaunch")}
            enabled={has("randomize")}
            onToggle={toggle}
          >
            <OnOff value={randomize} onChange={setRandomize} />
          </Row>
          <Row
            field="restoreSession"
            label={t("fingerprint.restoreSession")}
            enabled={has("restoreSession")}
            onToggle={toggle}
          >
            <OnOff value={restoreSession} onChange={setRestoreSession} />
          </Row>
          <Row
            field="newDevice"
            label={t("bulkEdit.newDevice")}
            enabled={has("newDevice")}
            onToggle={toggle}
          >
            <p className="text-xs text-muted-foreground">
              {t("bulkEdit.newDeviceDescription")}
            </p>
          </Row>
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose} disabled={isSaving}>
            {t("common.buttons.cancel")}
          </Button>
          <Button
            data-slot="bulk-edit-apply"
            onClick={() => void apply()}
            disabled={isSaving || enabled.size === 0 || blocked}
          >
            {t("bulkEdit.apply", { count: profileIds.length })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
