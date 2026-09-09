"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { translateBackendError } from "@/lib/backend-errors";
import { isFirstHopEncrypted, pickParsedProxy } from "@/lib/proxy-string";
import { canonicalProxyType } from "@/lib/proxy-type";
import type { ProxyParseResult, StoredProxy } from "@/types";
import { RippleButton } from "./ui/ripple";

interface ProxyFormData {
  name: string;
  proxy_type: string;
  host: string;
  port: number;
  username: string;
  password: string;
  vless_uri: string;
}

interface ProxyFormDialogProps {
  isOpen: boolean;
  onClose: () => void;
  editingProxy?: StoredProxy | null;
}

const DEFAULT_FORM: ProxyFormData = {
  name: "",
  proxy_type: "http",
  host: "",
  port: 8080,
  username: "",
  password: "",
  vless_uri: "",
};

/**
 * The type list, split by what the hop from this machine to the proxy actually
 * does on the wire rather than presented as one flat menu.
 *
 * A flat list rendered `HTTPS` next to `HTTP` and let it read as "the encrypted
 * one", which is not what Donut dials: `https` is a provider label on a
 * plaintext CONNECT endpoint. The stored value is untouched, it is the URL
 * scheme the Rust worker matches on, only the grouping and the label change.
 *
 * Shadowsocks has a heading of its own rather than sitting under the encrypted
 * one. Its hop is only as encrypted as its cipher, and the cipher is empty the
 * instant the type is picked, so "First hop encrypted" promised something the
 * note directly below the Select then denied, both readable in one glance. A
 * heading that says the cipher decides is the true one, and it stays true for
 * the `none` cipher that the encrypted heading never covered either.
 */
const ALWAYS_ENCRYPTED_FIRST_HOP_TYPES = ["httpstls", "vless"] as const;
const CIPHER_DEPENDENT_FIRST_HOP_TYPES = ["ss"] as const;
const PLAINTEXT_FIRST_HOP_TYPES = [
  "http",
  "https",
  "socks4",
  "socks5",
] as const;

const TYPE_GROUPS = [
  {
    labelKey: "proxies.form.firstHopGroupEncrypted",
    types: ALWAYS_ENCRYPTED_FIRST_HOP_TYPES,
  },
  {
    labelKey: "proxies.form.firstHopGroupCipher",
    types: CIPHER_DEPENDENT_FIRST_HOP_TYPES,
  },
  {
    labelKey: "proxies.form.firstHopGroupPlaintext",
    types: PLAINTEXT_FIRST_HOP_TYPES,
  },
] as const;

interface VlessEndpoint {
  host: string;
  port: number;
}

function parseVlessEndpoint(uri: string): VlessEndpoint | null {
  try {
    const parsed = new URL(uri.trim());
    const port = Number.parseInt(parsed.port, 10);
    if (
      parsed.protocol !== "vless:" ||
      !parsed.hostname ||
      !Number.isInteger(port) ||
      port < 1 ||
      port > 65535
    ) {
      return null;
    }

    const host =
      parsed.hostname.startsWith("[") && parsed.hostname.endsWith("]")
        ? parsed.hostname.slice(1, -1)
        : parsed.hostname;
    return { host, port };
  } catch {
    return null;
  }
}

export function ProxyFormDialog({
  isOpen,
  onClose,
  editingProxy,
}: ProxyFormDialogProps) {
  const { t } = useTranslation();
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [form, setForm] = useState<ProxyFormData>(DEFAULT_FORM);
  // The local parse only covers scheme/host/port. Whether Donut can actually
  // use the server — REALITY, XTLS Vision, plain TCP — is decided by the Rust
  // parser, so ask it (below) and show the specific reason while the user is
  // still editing rather than after they save. Declared here because
  // `handleSubmit` guards on it.
  const [vlessUnsupported, setVlessUnsupported] = useState<string | null>(null);

  const resetForm = useCallback(() => {
    setForm(DEFAULT_FORM);
  }, []);

  useEffect(() => {
    if (!isOpen) {
      return;
    }

    if (!editingProxy) {
      resetForm();
      return;
    }

    setForm({
      name: editingProxy.name,
      proxy_type: editingProxy.proxy_settings.proxy_type,
      host: editingProxy.proxy_settings.host,
      port: editingProxy.proxy_settings.port,
      username: editingProxy.proxy_settings.username ?? "",
      password: editingProxy.proxy_settings.password ?? "",
      vless_uri: editingProxy.proxy_settings.vless_uri ?? "",
    });
  }, [editingProxy, isOpen, resetForm]);

  const handleSubmit = useCallback(async () => {
    if (!form.name.trim()) {
      toast.error(t("proxies.form.nameRequired"));
      return;
    }

    const canonicalType = canonicalProxyType(form.proxy_type);
    const isVless = canonicalType === "vless";
    const vlessEndpoint = isVless ? parseVlessEndpoint(form.vless_uri) : null;

    if (isVless && !form.vless_uri.trim()) {
      toast.error(t("proxies.form.vlessUriRequired"));
      return;
    }

    if (isVless && !vlessEndpoint) {
      toast.error(t("proxies.form.vlessUriInvalid"));
      return;
    }

    if (isVless && vlessUnsupported) {
      toast.error(vlessUnsupported);
      return;
    }

    if (!isVless && (!form.host.trim() || !form.port)) {
      toast.error(t("proxies.form.hostPortRequired"));
      return;
    }

    if (
      canonicalType === "ss" &&
      (!form.username.trim() || !form.password.trim())
    ) {
      toast.error(t("proxies.form.ssCipherRequired"));
      return;
    }

    setIsSubmitting(true);
    try {
      const payload = {
        name: form.name.trim(),
        proxySettings: {
          proxy_type: form.proxy_type,
          host: vlessEndpoint?.host ?? form.host.trim(),
          port: vlessEndpoint?.port ?? form.port,
          username: isVless ? undefined : form.username.trim() || undefined,
          password: isVless ? undefined : form.password.trim() || undefined,
          vless_uri: isVless ? form.vless_uri.trim() : undefined,
        },
      };

      if (editingProxy) {
        await invoke("update_stored_proxy", {
          proxyId: editingProxy.id,
          ...payload,
        });
        toast.success(t("toasts.success.proxyUpdated"));
      } else {
        await invoke("create_stored_proxy", payload);
        toast.success(t("toasts.success.proxyCreated"));
      }

      onClose();
    } catch (error) {
      console.error("Failed to save proxy:", error);
      toast.error(
        t("proxies.form.saveFailed", {
          error: translateBackendError(t, error),
        }),
      );
    } finally {
      setIsSubmitting(false);
    }
  }, [editingProxy, form, onClose, t, vlessUnsupported]);

  const handleClose = useCallback(() => {
    if (!isSubmitting) {
      onClose();
    }
  }, [isSubmitting, onClose]);

  // Proxies are copied around as one string — `socks5://user:pass@host:1080`,
  // `host:1080:user:pass`, and a dozen variants of both — so a paste into any
  // one field is almost never meant for that field alone. Hand the clipboard to
  // the same Rust parser the import dialog uses and spread the result across
  // the form. The default paste is left alone until the answer comes back, so a
  // string that isn't a proxy (a hostname, a port) lands where it was dropped.
  const handleProxyPaste = useCallback(
    (event: React.ClipboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      const content = event.clipboardData.getData("text").trim();
      if (!content) {
        return;
      }

      // Captured before the browser applies the paste, so a proxy string
      // dropped into the empty name field names the proxy after its endpoint
      // instead of keeping the raw line.
      const nameBeforePaste = form.name.trim();

      void invoke<ProxyParseResult[]>("parse_txt_proxies", { content })
        .then((results) => {
          const parsed = pickParsedProxy(results);
          if (!parsed) {
            return;
          }
          setForm((previous) => ({
            ...previous,
            name: nameBeforePaste || `${parsed.host}:${parsed.port}`,
            proxy_type: parsed.proxy_type,
            host: parsed.host,
            port: parsed.port,
            username: parsed.username ?? "",
            password: parsed.password ?? "",
            vless_uri: parsed.vless_uri ?? "",
          }));
        })
        .catch((error: unknown) => {
          console.error("Failed to parse pasted proxy:", error);
        });
    },
    [form.name],
  );

  // The stored spelling is not the UI's to assume: a Shadowsocks proxy created
  // through the REST API arrives as `shadowsocks`, and every branch that asked
  // `=== "ss"` skipped it. Derive the type once and compare against that.
  const canonicalType = canonicalProxyType(form.proxy_type);
  const isVless = canonicalType === "vless";
  const isShadowsocks = canonicalType === "ss";
  const vlessEndpoint = isVless ? parseVlessEndpoint(form.vless_uri) : null;
  // The cipher decides for Shadowsocks, and this form keeps it in `username`.
  // Asked with `canonicalType`, not the raw stored spelling, so the answer
  // cannot disagree with the field labels two lines below: a REST-stored
  // `"ss "` was trimmed for the labels and not for this, and the form called a
  // proxy with a real cipher unencrypted while calling its field "Cipher".
  const firstHopEncrypted = isFirstHopEncrypted(canonicalType, form.username);
  // Shadowsocks is the one type whose hop is only as encrypted as its cipher,
  // and the cipher is empty until the user fills it in. Saying "not encrypted"
  // there states as settled something nobody has chosen yet, right under a
  // heading about encryption. Nothing downstream reads this: `firstHopEncrypted`
  // stays false, so every guard still fails closed on an empty cipher.
  const cipherUndecided = isShadowsocks && form.username.trim().length === 0;
  // What a filled field on a plaintext hop actually exposes, which is not the
  // same thing for every protocol: for the credentialed types it is the
  // username and password (see the payload built in handleSubmit), and for
  // Shadowsocks, whose password never touches the wire and whose field here is
  // the cipher, it is the destination and the payload. Same condition, two
  // different truths, so the panel below picks its sentence from the type.
  const showPlaintextExposure =
    !isVless && !firstHopEncrypted && form.username.trim().length > 0;
  // Radix matches an item by its value, so the item standing for this proxy
  // carries the proxy's own spelling. Without it a stored `shadowsocks` left
  // the trigger on its placeholder, and picking the visible Shadowsocks entry
  // to clear that silently retyped the proxy to `ss`.
  const typeItemValue = (type: string) =>
    type === canonicalType ? form.proxy_type : type;

  const trimmedVlessUri = form.vless_uri.trim();
  useEffect(() => {
    if (!isVless || trimmedVlessUri.length === 0) {
      setVlessUnsupported(null);
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void invoke("validate_vless_uri", { uri: trimmedVlessUri })
        .then(() => {
          if (!cancelled) setVlessUnsupported(null);
        })
        .catch((error: unknown) => {
          if (!cancelled) setVlessUnsupported(translateBackendError(t, error));
        });
    }, 300);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [isVless, trimmedVlessUri, t]);

  const hasInvalidVlessUri =
    isVless &&
    trimmedVlessUri.length > 0 &&
    (!vlessEndpoint || vlessUnsupported !== null);
  const isFormValid =
    form.name.trim() &&
    (isVless
      ? vlessEndpoint !== null && vlessUnsupported === null
      : form.host.trim() &&
        form.port > 0 &&
        form.port <= 65535 &&
        (!isShadowsocks || (form.username.trim() && form.password.trim())));

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            {editingProxy ? t("proxies.edit") : t("proxies.add")}
          </DialogTitle>
        </DialogHeader>

        <div className="@container grid gap-4 py-4">
          <div className="grid gap-2">
            <Label htmlFor="proxy-name">{t("proxies.form.name")}</Label>
            <Input
              id="proxy-name"
              value={form.name}
              onChange={(e) => {
                setForm({ ...form, name: e.target.value });
              }}
              onPaste={handleProxyPaste}
              placeholder={t("proxies.form.namePlaceholder")}
              disabled={isSubmitting}
            />
          </div>

          <div className="grid gap-2">
            <Label htmlFor="proxy-type">{t("proxies.form.type")}</Label>
            <Select
              value={form.proxy_type}
              onValueChange={(value) => {
                setForm({ ...form, proxy_type: value });
              }}
              disabled={isSubmitting}
            >
              <SelectTrigger
                id="proxy-type"
                aria-describedby={
                  canonicalType === "httpstls"
                    ? "proxy-type-first-hop proxy-type-tls-hint"
                    : "proxy-type-first-hop"
                }
              >
                <SelectValue placeholder={t("proxies.form.selectType")} />
              </SelectTrigger>
              <SelectContent>
                {TYPE_GROUPS.map((group) => (
                  <SelectGroup key={group.labelKey}>
                    <SelectLabel>{t(group.labelKey)}</SelectLabel>
                    {group.types.map((type) => (
                      <SelectItem key={type} value={typeItemValue(type)}>
                        {t(`proxies.types.${type}`)}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                ))}
              </SelectContent>
            </Select>
            <p
              id="proxy-type-first-hop"
              className={
                firstHopEncrypted
                  ? "text-xs text-muted-foreground"
                  : "text-xs text-warning-text"
              }
            >
              {firstHopEncrypted
                ? t("proxies.form.firstHopEncryptedNote")
                : cipherUndecided
                  ? t("proxies.form.firstHopCipherNote")
                  : t("proxies.form.firstHopPlaintextNote")}
            </p>
            {canonicalType === "httpstls" && (
              <p
                id="proxy-type-tls-hint"
                className="text-xs text-muted-foreground"
              >
                {t("proxies.form.httpsTlsHint")}
              </p>
            )}
          </div>

          {isVless ? (
            <div className="grid gap-2">
              <Label htmlFor="proxy-vless-uri">
                {t("proxies.form.vlessUri")}
              </Label>
              <Textarea
                id="proxy-vless-uri"
                value={form.vless_uri}
                onChange={(e) => {
                  setForm({ ...form, vless_uri: e.target.value });
                }}
                onPaste={handleProxyPaste}
                placeholder={t("proxies.form.vlessUriPlaceholder")}
                disabled={isSubmitting}
                aria-invalid={hasInvalidVlessUri}
                aria-describedby="proxy-vless-uri-help"
                autoCapitalize="none"
                autoComplete="off"
                spellCheck={false}
                className="min-h-24 resize-y font-mono text-xs leading-relaxed"
              />
              <p
                id="proxy-vless-uri-help"
                className={
                  hasInvalidVlessUri
                    ? "text-xs text-destructive-text"
                    : "text-xs text-muted-foreground"
                }
                role={hasInvalidVlessUri ? "alert" : undefined}
              >
                {hasInvalidVlessUri
                  ? (vlessUnsupported ?? t("proxies.form.vlessUriInvalid"))
                  : t("proxies.form.vlessUriHint")}
              </p>
            </div>
          ) : (
            <>
              <div className="grid grid-cols-2 gap-4">
                <div className="grid gap-2">
                  <Label htmlFor="proxy-host">{t("proxies.form.host")}</Label>
                  <Input
                    id="proxy-host"
                    value={form.host}
                    onChange={(e) => {
                      setForm({ ...form, host: e.target.value });
                    }}
                    onPaste={handleProxyPaste}
                    placeholder={t("proxies.form.hostPlaceholder")}
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="proxy-port">{t("proxies.form.port")}</Label>
                  <Input
                    id="proxy-port"
                    type="number"
                    value={form.port}
                    onChange={(e) => {
                      setForm({
                        ...form,
                        port: Number.parseInt(e.target.value, 10) || 0,
                      });
                    }}
                    onPaste={handleProxyPaste}
                    placeholder={t("proxies.form.portPlaceholder")}
                    min="1"
                    max="65535"
                    disabled={isSubmitting}
                  />
                </div>
              </div>

              <div className="grid grid-cols-1 gap-4 @sm:grid-cols-2">
                <div className="grid gap-2">
                  <Label htmlFor="proxy-username">
                    {isShadowsocks
                      ? t("proxies.form.cipher")
                      : t("proxies.form.username")}
                  </Label>
                  <Input
                    id="proxy-username"
                    value={form.username}
                    onChange={(e) => {
                      setForm({ ...form, username: e.target.value });
                    }}
                    placeholder={
                      isShadowsocks
                        ? t("proxies.form.cipherPlaceholder")
                        : t("proxies.form.usernamePlaceholder")
                    }
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="proxy-password">
                    {t("proxies.form.password")}
                  </Label>
                  <Input
                    id="proxy-password"
                    type="password"
                    value={form.password}
                    onChange={(e) => {
                      setForm({ ...form, password: e.target.value });
                    }}
                    placeholder={t("proxies.form.passwordPlaceholder")}
                    disabled={isSubmitting}
                  />
                </div>
              </div>

              {showPlaintextExposure && (
                <div className="space-y-2 rounded-md border border-warning/50 bg-warning/10 p-3">
                  <p className="font-medium">
                    {isShadowsocks
                      ? t("proxies.form.nullCipherHeading")
                      : t("proxies.form.credentialsInClearHeading")}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {isShadowsocks
                      ? t("proxies.form.nullCipherBody")
                      : t("proxies.form.credentialsInClearBody")}
                  </p>
                </div>
              )}
            </>
          )}
        </div>

        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={handleClose}
            disabled={isSubmitting}
          >
            {t("common.buttons.cancel")}
          </RippleButton>
          <LoadingButton
            isLoading={isSubmitting}
            onClick={handleSubmit}
            disabled={!isFormValid}
          >
            {editingProxy ? t("proxies.edit") : t("proxies.add")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
