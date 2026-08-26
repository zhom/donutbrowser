"use client";

import type { TFunction } from "i18next";
import { useCallback, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuTriangleAlert } from "react-icons/lu";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import type {
  CookieAnalysis,
  CookieIssue,
  CookiePasteFormat,
  CookieWriteMode,
  PastedCookiePreview,
} from "@/types";

/** Issue codes `cookie_paste.rs` can emit, mapped to their translation key. */
const ISSUE_KEYS: Record<string, string> = {
  EMPTY_INPUT: "emptyInput",
  SITE_INVALID: "siteInvalid",
  UNRECOGNIZED_FORMAT: "unrecognizedFormat",
  SITE_REQUIRED: "siteRequired",
  NO_COOKIES_FOUND: "noCookiesFound",
  NAME_EMPTY: "nameEmpty",
  NAME_INVALID: "nameInvalid",
  NAME_MISSING: "nameMissing",
  VALUE_INVALID: "valueInvalid",
  VALUE_COERCED: "valueCoerced",
  DOMAIN_FROM_SITE: "domainFromSite",
  DOMAIN_MISSING: "domainMissing",
  DOMAIN_INVALID: "domainInvalid",
  DOMAIN_ATTRIBUTE_IGNORED: "domainAttributeIgnored",
  HOST_ONLY_MISMATCH: "hostOnlyMismatch",
  PATH_REPAIRED: "pathRepaired",
  EXPIRY_MILLISECONDS: "expiryMilliseconds",
  EXPIRY_CLAMPED: "expiryClamped",
  EXPIRY_INVALID: "expiryInvalid",
  EXPIRES_INVALID: "expiresInvalid",
  MAX_AGE_INVALID: "maxAgeInvalid",
  MAX_AGE_DELETION: "maxAgeDeletion",
  SAME_SITE_NONE_INSECURE: "sameSiteNoneInsecure",
  SAME_SITE_UNRECOGNIZED: "sameSiteUnrecognized",
  DUPLICATE_COOKIE: "duplicateCookie",
  BOOL_COERCED_FROM_STRING: "boolCoercedFromString",
  BOOL_INVALID: "boolInvalid",
  QUOTED_VALUE: "quotedValue",
  JSON_PARSE_FAILED: "jsonParseFailed",
  JSON_NOT_COOKIE_LIST: "jsonNotCookieList",
  JSON_ENTRY_NOT_OBJECT: "jsonEntryNotObject",
  NETSCAPE_PATH_OMITTED: "netscapePathOmitted",
  NETSCAPE_FIELD_COUNT: "netscapeFieldCount",
  NETSCAPE_INCLUDE_SUBDOMAINS_INVALID: "netscapeIncludeSubdomainsInvalid",
  NETSCAPE_SECURE_INVALID: "netscapeSecureInvalid",
  NETSCAPE_EXPIRY_INVALID: "netscapeExpiryInvalid",
  NAME_VALUE_NO_PAIR: "nameValueNoPair",
  PAIR_TREATED_AS_ATTRIBUTE: "pairTreatedAsAttribute",
};

const FORMAT_KEYS: Record<CookiePasteFormat, string> = {
  json: "cookies.paste.formatJson",
  netscape: "cookies.paste.formatNetscape",
  nameValue: "cookies.paste.formatNameValue",
};

const VISIBLE_ISSUES = 5;

const RELATIVE_UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["year", 31_536_000],
  ["month", 2_592_000],
  ["day", 86_400],
  ["hour", 3600],
  ["minute", 60],
];

export interface CookiePastePanelProps {
  content: string;
  onContentChange: (content: string) => void;
  site: string;
  onSiteChange: (site: string) => void;
  mode: CookieWriteMode;
  onModeChange: (mode: CookieWriteMode) => void;
  includeExpired: boolean;
  onIncludeExpiredChange: (includeExpired: boolean) => void;
  analysis: CookieAnalysis | null;
  isAnalyzing: boolean;
  disabled: boolean;
}

export function CookiePastePanel({
  content,
  onContentChange,
  site,
  onSiteChange,
  mode,
  onModeChange,
  includeExpired,
  onIncludeExpiredChange,
  analysis,
  isAnalyzing,
  disabled,
}: CookiePastePanelProps) {
  const { t, i18n } = useTranslation();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [showAllIssues, setShowAllIssues] = useState(false);
  const [fileError, setFileError] = useState<string | null>(null);

  const relativeFormatter = useMemo(
    () => new Intl.RelativeTimeFormat(i18n.language, { numeric: "auto" }),
    [i18n.language],
  );

  const readFile = useCallback(
    (file: File) => {
      const reader = new FileReader();
      reader.onload = (event) => {
        setFileError(null);
        onContentChange(String(event.target?.result ?? ""));
      };
      reader.onerror = () => {
        setFileError(t("cookies.management.fileReadError"));
      };
      reader.readAsText(file);
    },
    [onContentChange, t],
  );

  const handleDrop = useCallback(
    (event: React.DragEvent<HTMLTextAreaElement>) => {
      const file = event.dataTransfer.files[0];
      if (!file) return;
      event.preventDefault();
      readFile(file);
    },
    [readFile],
  );

  // A JSON or Netscape entry with no domain of its own needs the site as much
  // as a bare pair does: without the field, "carries no domain and no site was
  // given" is a dead end with no control anywhere that answers it. The last
  // clause keeps the field once anything has been typed into it, because every
  // other condition stops being true the moment the site is accepted, which
  // would yank the input out from under the cursor.
  const siteVisible =
    analysis !== null &&
    (analysis.siteRequired ||
      analysis.format === "nameValue" ||
      analysis.issues.some((issue) => issue.code === "DOMAIN_MISSING") ||
      site.trim() !== "");

  const scopeDomains = useMemo(() => {
    if (!analysis) return [];
    return [...new Set(analysis.cookies.map((cookie) => cookie.domain))];
  }, [analysis]);

  const issues = analysis?.issues ?? [];
  const shownIssues = showAllIssues ? issues : issues.slice(0, VISIBLE_ISSUES);

  const formatExpiry = useCallback(
    (expires: number) => {
      if (expires === 0) return t("cookies.paste.session");
      const delta = expires - Math.floor(Date.now() / 1000);
      for (const [unit, seconds] of RELATIVE_UNITS) {
        if (Math.abs(delta) >= seconds) {
          return relativeFormatter.format(Math.trunc(delta / seconds), unit);
        }
      }
      return relativeFormatter.format(delta, "second");
    },
    [relativeFormatter, t],
  );

  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <Label htmlFor="cookie-paste-input">{t("cookies.paste.label")}</Label>
        <Textarea
          id="cookie-paste-input"
          rows={10}
          spellCheck={false}
          disabled={disabled}
          value={content}
          placeholder={t("cookies.paste.placeholder")}
          className="resize-y font-mono text-xs"
          onChange={(event) => {
            setFileError(null);
            onContentChange(event.target.value);
          }}
          onDrop={handleDrop}
          onDragOver={(event) => {
            if (event.dataTransfer.types.includes("Files")) {
              event.preventDefault();
            }
          }}
        />
        <div className="flex flex-wrap items-center justify-between gap-2">
          <button
            type="button"
            disabled={disabled}
            className="text-xs text-muted-foreground underline-offset-2 transition-colors hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
            onClick={() => fileInputRef.current?.click()}
          >
            {t("cookies.paste.chooseFile")}
          </button>
          {isAnalyzing ? (
            <span className="text-xs text-muted-foreground">
              {t("cookies.paste.analyzing")}
            </span>
          ) : (
            analysis !== null &&
            content.trim() !== "" && (
              <Badge
                variant={analysis.format ? "secondary" : "destructive"}
                className="font-normal"
              >
                {analysis.format
                  ? t(FORMAT_KEYS[analysis.format])
                  : t("cookies.paste.formatUnknown")}
              </Badge>
            )
          )}
          <input
            ref={fileInputRef}
            type="file"
            accept=".txt,.cookies,.json"
            className="hidden"
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) readFile(file);
              event.target.value = "";
            }}
          />
        </div>
        {fileError && (
          <p className="text-xs text-destructive-text">{fileError}</p>
        )}
      </div>

      {siteVisible && (
        <div className="space-y-2">
          <Label htmlFor="cookie-paste-site">
            {t("cookies.paste.siteLabel")}
          </Label>
          <Input
            id="cookie-paste-site"
            disabled={disabled}
            value={site}
            placeholder={t("cookies.paste.sitePlaceholder")}
            onChange={(event) => {
              onSiteChange(event.target.value);
            }}
          />
          <p className="text-xs text-muted-foreground">
            {t("cookies.paste.siteHelp")}
          </p>
          {scopeDomains.map((domain) => (
            <p key={domain} className="text-xs text-foreground">
              {domain.startsWith(".")
                ? t("cookies.paste.scopeSubdomains", { domain })
                : t("cookies.paste.scopeHostOnly", { domain })}
            </p>
          ))}
        </div>
      )}

      <div className="space-y-2">
        <Label>{t("common.labels.mode")}</Label>
        <RadioGroup
          value={mode}
          disabled={disabled}
          onValueChange={(value) => {
            onModeChange(value as CookieWriteMode);
          }}
        >
          <label
            htmlFor="cookie-mode-merge"
            className="flex cursor-pointer items-start gap-2"
          >
            <RadioGroupItem
              id="cookie-mode-merge"
              value="merge"
              className="mt-0.5"
            />
            <span className="space-y-0.5">
              <span className="block text-sm font-medium">
                {t("cookies.paste.modeMerge")}
              </span>
              <span className="block text-xs text-muted-foreground">
                {t("cookies.paste.modeMergeDesc")}
              </span>
            </span>
          </label>
          <label
            htmlFor="cookie-mode-replace"
            className="flex cursor-pointer items-start gap-2"
          >
            <RadioGroupItem
              id="cookie-mode-replace"
              value="replaceMatchingSites"
              className="mt-0.5"
            />
            <span className="space-y-0.5">
              <span className="block text-sm font-medium">
                {t("cookies.paste.modeReplace")}
              </span>
              <span className="block text-xs text-muted-foreground">
                {t("cookies.paste.modeReplaceDesc")}
              </span>
              {analysis && (
                <span className="block text-xs text-warning-text">
                  {t("cookies.paste.replaceDeleteCount", {
                    n:
                      analysis.replaceDeleteCount === null
                        ? t("cookies.paste.unknownCount")
                        : String(analysis.replaceDeleteCount),
                  })}
                </span>
              )}
            </span>
          </label>
        </RadioGroup>
      </div>

      {analysis !== null && analysis.expiredCount > 0 && (
        <label
          htmlFor="cookie-include-expired"
          className="flex cursor-pointer items-start gap-2"
        >
          <Checkbox
            id="cookie-include-expired"
            disabled={disabled}
            checked={includeExpired}
            onCheckedChange={(checked) => {
              onIncludeExpiredChange(checked === true);
            }}
            className="mt-0.5"
          />
          <span className="space-y-0.5">
            <span className="block text-sm">
              {t("cookies.paste.includeExpired")}
            </span>
            <span className="block text-xs text-muted-foreground">
              {t("cookies.paste.expiredNote", {
                n: analysis.expiredCount,
              })}
            </span>
          </span>
        </label>
      )}

      {analysis?.clearsOnClose && (
        <Alert>
          <LuTriangleAlert className="text-warning-text" />
          <AlertDescription>
            {t("cookies.paste.clearsOnCloseWarning")}
          </AlertDescription>
        </Alert>
      )}

      {analysis !== null && analysis.cookies.length > 0 && (
        <div className="space-y-2">
          <Label>
            {t("cookies.paste.previewTitle", { n: analysis.cookies.length })}
          </Label>
          <Table
            containerClassName="max-h-[clamp(120px,28vh,340px)] overflow-y-auto rounded-md border"
            className="text-xs"
          >
            <TableHeader className="sticky top-0 bg-background">
              <TableRow>
                <TableHead>{t("cookies.paste.colSite")}</TableHead>
                <TableHead>{t("cookies.paste.colName")}</TableHead>
                <TableHead>{t("cookies.paste.colPath")}</TableHead>
                <TableHead>{t("cookies.paste.colExpires")}</TableHead>
                <TableHead>{t("cookies.paste.colSecure")}</TableHead>
                <TableHead>{t("cookies.paste.colHttpOnly")}</TableHead>
                <TableHead>{t("cookies.paste.colSameSite")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {analysis.cookies.map((cookie) => (
                <CookiePreviewRow
                  key={`${cookie.domain}|${cookie.path}|${cookie.name}`}
                  cookie={cookie}
                  expiresLabel={formatExpiry(cookie.expires)}
                />
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      {issues.length > 0 && (
        <div className="space-y-2">
          <Label>{t("cookies.paste.issuesTitle")}</Label>
          <div className="space-y-1">
            {shownIssues.map((issue, index) => (
              <IssueRow
                key={`${issue.code}-${issue.source ?? ""}-${index}`}
                issue={issue}
              />
            ))}
          </div>
          {issues.length > VISIBLE_ISSUES && (
            <button
              type="button"
              className="text-xs text-muted-foreground transition-colors hover:text-foreground"
              onClick={() => {
                setShowAllIssues((previous) => !previous);
              }}
            >
              {showAllIssues
                ? t("cookies.paste.showFewer")
                : t("cookies.paste.showAll", { n: issues.length })}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function CookiePreviewRow({
  cookie,
  expiresLabel,
}: {
  cookie: PastedCookiePreview;
  expiresLabel: string;
}) {
  const { t } = useTranslation();
  const sameSite =
    cookie.sameSite === 2
      ? t("cookies.paste.sameSiteStrict")
      : cookie.sameSite === 1
        ? t("cookies.paste.sameSiteLax")
        : cookie.sameSite === 0
          ? t("cookies.paste.sameSiteNone")
          : t("cookies.paste.sameSiteUnspecified");

  return (
    <TableRow>
      <TableCell className="font-mono whitespace-nowrap">
        {cookie.domain}
      </TableCell>
      <TableCell className="font-mono whitespace-nowrap">
        {cookie.name}
      </TableCell>
      <TableCell className="font-mono whitespace-nowrap">
        {cookie.path}
      </TableCell>
      <TableCell className="whitespace-nowrap">{expiresLabel}</TableCell>
      <TableCell className="whitespace-nowrap">
        {cookie.isSecure ? t("cookies.paste.yes") : t("cookies.paste.no")}
      </TableCell>
      <TableCell className="whitespace-nowrap">
        {cookie.isHttpOnly ? t("cookies.paste.yes") : t("cookies.paste.no")}
      </TableCell>
      <TableCell className="whitespace-nowrap">{sameSite}</TableCell>
    </TableRow>
  );
}

export function IssueRow({ issue }: { issue: CookieIssue }) {
  const { t } = useTranslation();
  const key = ISSUE_KEYS[issue.code];
  const message = key
    ? t(`cookies.paste.issues.${key}`, issue.params)
    : t("cookies.paste.issues.unknown", { code: issue.code });

  const tone =
    issue.severity === "error"
      ? "text-destructive-text"
      : issue.severity === "warning"
        ? "text-warning-text"
        : "text-muted-foreground";

  return (
    <p className={`text-xs ${tone}`}>
      {issue.source && (
        <span className="text-muted-foreground">
          {formatIssueSource(t, issue.source)}
          {": "}
        </span>
      )}
      {message}
    </p>
  );
}

/**
 * `source` arrives as `line 4` / `cookie 12`, built in Rust where there is no
 * translator. Recognise those two shapes so the prefix is localised too.
 */
function formatIssueSource(t: TFunction, source: string): string {
  const match = /^(line|cookie) (\d+)$/.exec(source);
  if (!match) return source;
  return match[1] === "line"
    ? t("cookies.paste.sourceLine", { n: match[2] })
    : t("cookies.paste.sourceCookie", { n: match[2] });
}
