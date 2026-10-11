"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { LuTrash2 } from "react-icons/lu";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { Button } from "@/components/ui/button";
import { CopyToClipboard } from "@/components/ui/copy-to-clipboard";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { translateBackendError } from "@/lib/backend-errors";
import { cn } from "@/lib/utils";

/** `TotpCode` in `src-tauri/src/totp.rs`. */
interface TotpCode {
  code: string;
  valid_until: number;
  period: number;
}

/** `TotpEntry` in `src-tauri/src/profile_totp.rs`: everything but the key. */
interface TotpEntry {
  id: string;
  label: string;
  issuer: string | null;
  account: string | null;
  digits: number;
  period: number;
}

type TotpEntryCode = TotpCode & { id: string };

/** Seconds since the epoch, ticking once a second. */
function useNow(): number {
  const [now, setNow] = React.useState(() => Math.floor(Date.now() / 1000));
  React.useEffect(() => {
    const timer = window.setInterval(() => {
      setNow(Math.floor(Date.now() / 1000));
    }, 1000);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}

/** Groups of three, the way authenticator apps show a code. */
function spaced(code: string): string {
  return code.length > 6
    ? `${code.slice(0, 4)} ${code.slice(4)}`
    : `${code.slice(0, 3)} ${code.slice(3)}`;
}

function CodeView({ code, now }: { code: TotpCode; now: number }) {
  const { t } = useTranslation();
  const remaining = Math.max(0, code.valid_until - now);
  return (
    <div className="flex items-center gap-3" data-slot="totp-code">
      <span className="font-mono text-2xl tracking-wider tabular-nums">
        {spaced(code.code)}
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="h-1 overflow-hidden rounded-full bg-foreground/8">
          <div
            className={cn(
              "h-full rounded-full transition-[width] duration-1000 ease-linear",
              remaining <= 5 ? "bg-warning" : "bg-primary",
            )}
            style={{ width: `${(remaining / code.period) * 100}%` }}
          />
        </div>
        <span className="text-xs text-muted-foreground tabular-nums">
          {t("totp.expiresIn", { seconds: remaining })}
        </span>
      </div>
      <CopyToClipboard
        text={code.code}
        size="sm"
        variant="soft"
        successMessage={t("totp.copied")}
      />
    </div>
  );
}

/**
 * Turn a pasted key into a code without keeping it: the key lives in this
 * dialog's state and nowhere else.
 */
export function TotpToolDialog({
  isOpen,
  onClose,
}: {
  isOpen: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const now = useNow();
  const [key, setKey] = React.useState("");
  const [code, setCode] = React.useState<TotpCode | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!isOpen) {
      setKey("");
      setCode(null);
      setError(null);
    }
  }, [isOpen]);

  const expired = code !== null && now >= code.valid_until;
  React.useEffect(() => {
    if (!key.trim()) {
      setCode(null);
      setError(null);
      return;
    }
    if (code && !expired) return;
    let current = true;
    invoke<TotpCode>("generate_totp_code", { key })
      .then((next) => {
        if (!current) return;
        setCode(next);
        setError(null);
      })
      .catch((err: unknown) => {
        if (!current) return;
        setCode(null);
        setError(translateBackendError(t, err));
      });
    return () => {
      current = false;
    };
  }, [key, expired, code, t]);

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-md" data-slot="totp-tool-dialog">
        <DialogHeader>
          <DialogTitle>{t("totp.toolTitle")}</DialogTitle>
          <DialogDescription>{t("totp.toolDescription")}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-2">
          <Label htmlFor="totp-tool-key">{t("totp.keyLabel")}</Label>
          <Input
            id="totp-tool-key"
            type="password"
            value={key}
            onChange={(event) => {
              setKey(event.target.value);
              setCode(null);
            }}
            placeholder={t("totp.keyPlaceholder")}
            autoComplete="off"
            spellCheck={false}
          />
          {error && <p className="text-xs text-destructive-text">{error}</p>}
          {code && <CodeView code={code} now={now} />}
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** The profile's saved 2FA keys, shown as live codes. */
export function ProfileTotpSection({
  profileId,
  isDisabled,
}: {
  profileId: string;
  isDisabled?: boolean;
}) {
  const { t } = useTranslation();
  const now = useNow();
  const [entries, setEntries] = React.useState<TotpEntry[]>([]);
  const [codes, setCodes] = React.useState<Map<string, TotpEntryCode>>(
    new Map(),
  );
  const [label, setLabel] = React.useState("");
  const [key, setKey] = React.useState("");
  const [isAdding, setIsAdding] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [deleting, setDeleting] = React.useState<TotpEntry | null>(null);

  const load = React.useCallback(async () => {
    try {
      const [nextEntries, nextCodes] = await Promise.all([
        invoke<TotpEntry[]>("list_profile_totp", { profileId }),
        invoke<TotpEntryCode[]>("get_profile_totp_codes", { profileId }),
      ]);
      setEntries(nextEntries);
      setCodes(new Map(nextCodes.map((code) => [code.id, code])));
    } catch (err) {
      setError(translateBackendError(t, err));
    }
  }, [profileId, t]);

  React.useEffect(() => {
    void load();
  }, [load]);

  const anyExpired = [...codes.values()].some(
    (code) => now >= code.valid_until,
  );
  React.useEffect(() => {
    if (!anyExpired) return;
    void invoke<TotpEntryCode[]>("get_profile_totp_codes", { profileId })
      .then((next) => setCodes(new Map(next.map((code) => [code.id, code]))))
      .catch(() => {});
  }, [anyExpired, profileId]);

  const add = async () => {
    setIsAdding(true);
    setError(null);
    try {
      await invoke("add_profile_totp", { profileId, label, key });
      setLabel("");
      setKey("");
      await load();
    } catch (err) {
      setError(translateBackendError(t, err));
    } finally {
      setIsAdding(false);
    }
  };

  const confirmDelete = async () => {
    if (!deleting) return;
    try {
      await invoke("delete_profile_totp", {
        profileId,
        entryId: deleting.id,
      });
      await load();
    } catch (err) {
      setError(translateBackendError(t, err));
    } finally {
      setDeleting(null);
    }
  };

  return (
    <div className="flex flex-col gap-3" data-slot="profile-totp-section">
      <p className="text-xs text-muted-foreground">
        {t("totp.sectionDescription")}
      </p>
      {entries.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("totp.empty")}</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {entries.map((entry) => {
            const code = codes.get(entry.id);
            return (
              <li
                key={entry.id}
                className="flex flex-col gap-1 rounded-lg bg-foreground/4 p-3"
                data-slot="profile-totp-entry"
              >
                <div className="flex items-center gap-2">
                  <span className="min-w-0 flex-1 truncate text-sm font-medium">
                    {entry.label}
                    {entry.account && entry.account !== entry.label && (
                      <span className="ml-2 text-xs font-normal text-muted-foreground">
                        {entry.account}
                      </span>
                    )}
                  </span>
                  <Button
                    type="button"
                    variant="subtle"
                    size="icon"
                    className="size-7"
                    aria-label={t("totp.delete")}
                    onClick={() => setDeleting(entry)}
                  >
                    <LuTrash2 className="size-3.5" />
                  </Button>
                </div>
                {code && <CodeView code={code} now={now} />}
              </li>
            );
          })}
        </ul>
      )}
      <form
        className="flex flex-col gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          void add();
        }}
      >
        <div className="grid gap-2 sm:grid-cols-[1fr_2fr]">
          <Input
            variant="soft"
            value={label}
            onChange={(event) => setLabel(event.target.value)}
            placeholder={t("totp.labelPlaceholder")}
            aria-label={t("totp.labelPlaceholder")}
            disabled={isDisabled || isAdding}
          />
          <Input
            variant="soft"
            type="password"
            data-slot="profile-totp-key"
            value={key}
            onChange={(event) => setKey(event.target.value)}
            placeholder={t("totp.keyPlaceholder")}
            aria-label={t("totp.keyLabel")}
            autoComplete="off"
            spellCheck={false}
            disabled={isDisabled || isAdding}
          />
        </div>
        {error && <p className="text-xs text-destructive-text">{error}</p>}
        <Button
          type="submit"
          size="sm"
          className="h-7 self-start text-xs"
          disabled={isDisabled || isAdding || !key.trim()}
        >
          {t("totp.add")}
        </Button>
      </form>
      <DeleteConfirmationDialog
        isOpen={deleting !== null}
        onClose={() => setDeleting(null)}
        onConfirm={confirmDelete}
        title={t("totp.deleteTitle", { label: deleting?.label ?? "" })}
        description={t("totp.deleteDescription")}
        confirmButtonText={t("common.buttons.delete")}
      />
    </div>
  );
}
