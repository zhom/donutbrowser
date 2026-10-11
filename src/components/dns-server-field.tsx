"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { LuLock, LuLockOpen, LuTriangleAlert } from "react-icons/lu";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { translateBackendError } from "@/lib/backend-errors";
import { cn } from "@/lib/utils";

/** `DnsServerSummary` in `src-tauri/src/profile_dns.rs`. */
interface DnsServerSummary {
  transport: "https" | "tls" | "tcp";
  server: string;
  encrypted: boolean;
  upgraded: boolean;
}

type DnsServerState =
  | { kind: "default" }
  | { kind: "valid"; summary: DnsServerSummary }
  | { kind: "invalid"; message: string };

/**
 * Encrypted resolvers that keep no query logs, offered as one click each.
 * Provider names, not copy: they are the same in every language.
 */
const DNS_SERVER_PRESETS = [
  { name: "Quad9", value: "https://dns.quad9.net/dns-query" },
  { name: "Mullvad", value: "https://dns.mullvad.net/dns-query" },
  { name: "Cloudflare", value: "https://cloudflare-dns.com/dns-query" },
] as const;

/** The backend's reading of `value`; `null` until the first answer. */
function useDnsServerState(value: string): DnsServerState | null {
  const { t } = useTranslation();
  const [state, setState] = React.useState<DnsServerState | null>(null);

  React.useEffect(() => {
    let current = true;
    const timer = window.setTimeout(
      () => {
        invoke<DnsServerSummary | null>("describe_dns_server", { value })
          .then((summary) => {
            if (!current) return;
            setState(
              summary ? { kind: "valid", summary } : { kind: "default" },
            );
          })
          .catch((error: unknown) => {
            if (!current) return;
            setState({
              kind: "invalid",
              message: translateBackendError(t, error),
            });
          });
      },
      value.trim() ? 200 : 0,
    );
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [value, t]);

  return state;
}

interface DnsServerFieldProps {
  id: string;
  value: string;
  onChange: (value: string) => void;
  /** False while the value cannot be saved. */
  onValidityChange?: (valid: boolean) => void;
  disabled?: boolean;
  /** Rendered after the input, for example a save button. */
  action?: React.ReactNode;
}

export function DnsServerField({
  id,
  value,
  onChange,
  onValidityChange,
  disabled,
  action,
}: DnsServerFieldProps) {
  const { t } = useTranslation();
  const statusId = React.useId();
  const state = useDnsServerState(value);
  const invalid = state?.kind === "invalid";

  React.useEffect(() => {
    onValidityChange?.(!invalid);
  }, [invalid, onValidityChange]);

  let tone = "text-muted-foreground";
  let icon: React.ReactNode = null;
  let message = "";
  if (state?.kind === "default") {
    message = t("dnsServer.summary.default");
  } else if (state?.kind === "invalid") {
    tone = "text-destructive-text";
    icon = <LuTriangleAlert className="mt-px size-3.5 shrink-0" />;
    message = state.message;
  } else if (state?.kind === "valid") {
    const { summary } = state;
    if (summary.encrypted) {
      icon = <LuLock className="mt-px size-3.5 shrink-0 text-success-text" />;
      message = t(
        summary.transport === "https"
          ? "dnsServer.summary.https"
          : "dnsServer.summary.tls",
        { server: summary.server },
      );
      if (summary.upgraded) {
        message = `${message} ${t("dnsServer.summary.upgraded")}`;
      }
    } else {
      tone = "text-warning-text";
      icon = <LuLockOpen className="mt-px size-3.5 shrink-0" />;
      message = t("dnsServer.summary.tcp", { server: summary.server });
    }
  }

  return (
    <div className="space-y-1.5" data-slot="dns-server-field">
      <div className="flex items-center gap-2">
        <Input
          id={id}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          placeholder={t("dnsServer.placeholder")}
          disabled={disabled}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          autoComplete="off"
          aria-invalid={invalid || undefined}
          aria-describedby={statusId}
        />
        {action}
      </div>
      <p
        id={statusId}
        aria-live="polite"
        data-slot="dns-server-status"
        data-state={state?.kind ?? "pending"}
        className={cn("flex min-h-4 items-start gap-1.5 text-xs", tone)}
      >
        {icon}
        <span>{message}</span>
      </p>
      <div className="flex flex-wrap items-center gap-1">
        <span className="text-xs text-muted-foreground">
          {t("dnsServer.presets")}
        </span>
        {DNS_SERVER_PRESETS.map((preset) => (
          <Button
            key={preset.value}
            type="button"
            variant={value.trim() === preset.value ? "soft" : "subtle"}
            size="sm"
            className="h-6 px-2 text-xs"
            aria-pressed={value.trim() === preset.value}
            disabled={disabled}
            onClick={() => onChange(preset.value)}
          >
            {preset.name}
          </Button>
        ))}
      </div>
    </div>
  );
}
