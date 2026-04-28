"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuUpload } from "react-icons/lu";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
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
import { ScrollArea } from "@/components/ui/scroll-area";
import { getCurrentOS } from "@/lib/browser-utils";
import type {
  ParsedProxyLine,
  ProxyImportResult,
  ProxyParseResult,
} from "@/types";
import { RippleButton } from "./ui/ripple";

interface ProxyImportDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

type ImportStep = "dropzone" | "preview" | "ambiguous" | "result";

interface AmbiguousProxy {
  line: string;
  possible_formats: string[];
  selectedFormat?: string;
}

export function ProxyImportDialog({ isOpen, onClose }: ProxyImportDialogProps) {
  const { t } = useTranslation();
  const [step, setStep] = useState<ImportStep>("dropzone");
  const [isDragOver, setIsDragOver] = useState(false);
  const [parsedProxies, setParsedProxies] = useState<ParsedProxyLine[]>([]);
  const [ambiguousProxies, setAmbiguousProxies] = useState<AmbiguousProxy[]>(
    [],
  );
  const [invalidProxies, setInvalidProxies] = useState<
    { line: string; reason: string }[]
  >([]);
  const [importResult, setImportResult] = useState<ProxyImportResult | null>(
    null,
  );
  const [isImporting, setIsImporting] = useState(false);
  const [namePrefix, setNamePrefix] = useState(
    t("proxies.importDialog.namePrefixDefault"),
  );

  const os = getCurrentOS();
  const modKey = os === "macos" ? "⌘" : "Ctrl";

  const resetState = useCallback(() => {
    setStep("dropzone");
    setIsDragOver(false);
    setParsedProxies([]);
    setAmbiguousProxies([]);
    setInvalidProxies([]);
    setImportResult(null);
    setIsImporting(false);
    setNamePrefix(t("proxies.importDialog.namePrefixDefault"));
  }, [t]);

  const processContent = useCallback(
    async (content: string, isJson: boolean, _filename = "") => {
      try {
        if (isJson) {
          setIsImporting(true);
          const result = await invoke<ProxyImportResult>(
            "import_proxies_json",
            {
              content,
            },
          );
          setImportResult(result);
          setStep("result");
          await emit("stored-proxies-changed");
        } else {
          const results = await invoke<ProxyParseResult[]>(
            "parse_txt_proxies",
            {
              content,
            },
          );

          const parsed: ParsedProxyLine[] = [];
          const ambiguous: AmbiguousProxy[] = [];
          const invalid: { line: string; reason: string }[] = [];

          for (const result of results) {
            if (result.status === "parsed") {
              parsed.push(result);
            } else if (result.status === "ambiguous") {
              ambiguous.push({
                line: result.line,
                possible_formats: result.possible_formats,
              });
            } else if (result.status === "invalid") {
              invalid.push({ line: result.line, reason: result.reason });
            }
          }

          setParsedProxies(parsed);
          setAmbiguousProxies(ambiguous);
          setInvalidProxies(invalid);

          if (ambiguous.length > 0) {
            setStep("ambiguous");
          } else if (parsed.length > 0) {
            setStep("preview");
          } else {
            toast.error(t("proxies.importDialog.noValidProxies"));
          }
        }
      } catch (error) {
        console.error("Failed to process content:", error);
        toast.error(
          error instanceof Error
            ? error.message
            : t("proxies.importDialog.fileProcessError"),
        );
      } finally {
        setIsImporting(false);
      }
    },
    [t],
  );

  const handleFileRead = useCallback(
    (file: File) => {
      const reader = new FileReader();
      reader.onload = (e) => {
        const content = e.target?.result as string;
        const isJson = file.name.endsWith(".json");
        void processContent(content, isJson, file.name);
      };
      reader.onerror = () => {
        toast.error(t("proxies.importDialog.fileReadError"));
      };
      reader.readAsText(file);
    },
    [processContent, t],
  );

  const handleDrop = useCallback(
    (e: React.DragEvent<HTMLDivElement>) => {
      e.preventDefault();
      setIsDragOver(false);

      const files = Array.from(e.dataTransfer.files);
      const validFile = files.find(
        (f) => f.name.endsWith(".json") || f.name.endsWith(".txt"),
      );

      if (validFile) {
        handleFileRead(validFile);
      } else {
        toast.error(t("proxies.importDialog.wrongFileType"));
      }
    },
    [handleFileRead, t],
  );

  const handleDragOver = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setIsDragOver(true);
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setIsDragOver(false);
  }, []);

  // Handle paste from clipboard
  useEffect(() => {
    if (!isOpen || step !== "dropzone") return;

    const handlePaste = (e: ClipboardEvent) => {
      const text = e.clipboardData?.getData("text");
      if (text) {
        // Try to detect if it's JSON
        const trimmed = text.trim();
        const isJson =
          (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
          (trimmed.startsWith("[") && trimmed.endsWith("]"));
        // Use "pasted.txt" as filename to trigger content-based detection
        void processContent(text, isJson, "pasted.txt");
      }
    };

    document.addEventListener("paste", handlePaste);
    return () => {
      document.removeEventListener("paste", handlePaste);
    };
  }, [isOpen, step, processContent]);

  const handleImport = useCallback(async () => {
    setIsImporting(true);
    try {
      const result = await invoke<ProxyImportResult>(
        "import_proxies_from_parsed",
        {
          parsedProxies,
          namePrefix:
            namePrefix.trim() || t("proxies.importDialog.namePrefixDefault"),
        },
      );
      setImportResult(result);
      setStep("result");
      await emit("stored-proxies-changed");
    } catch (error) {
      console.error("Failed to import proxies:", error);
      toast.error(
        error instanceof Error
          ? error.message
          : t("proxies.importDialog.failed"),
      );
    } finally {
      setIsImporting(false);
    }
  }, [parsedProxies, namePrefix, t]);

  const handleAmbiguousFormatSelect = useCallback(
    (index: number, format: string) => {
      setAmbiguousProxies((prev) =>
        prev.map((p, i) =>
          i === index ? { ...p, selectedFormat: format } : p,
        ),
      );
    },
    [],
  );

  const handleResolveAmbiguous = useCallback(() => {
    // Convert ambiguous proxies to parsed based on selected format
    const resolved: ParsedProxyLine[] = ambiguousProxies
      .filter((p) => p.selectedFormat)
      .map((p) => {
        const parts = p.line.split(":");
        if (p.selectedFormat === "host:port:username:password") {
          return {
            proxy_type: "http",
            host: parts[0],
            port: Number.parseInt(parts[1], 10),
            username: parts[2],
            password: parts[3],
            original_line: p.line,
          };
        }
        // username:password:host:port
        return {
          proxy_type: "http",
          host: parts[2],
          port: Number.parseInt(parts[3], 10),
          username: parts[0],
          password: parts[1],
          original_line: p.line,
        };
      });

    setParsedProxies((prev) => [...prev, ...resolved]);
    setStep("preview");
  }, [ambiguousProxies]);

  const handleClose = useCallback(() => {
    resetState();
    onClose();
  }, [resetState, onClose]);

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("proxies.importDialog.title")}</DialogTitle>
          <DialogDescription>
            {step === "dropzone" && t("proxies.importDialog.descDropzone")}
            {step === "preview" && t("proxies.importDialog.descPreview")}
            {step === "ambiguous" && t("proxies.importDialog.descAmbiguous")}
            {step === "result" && t("proxies.importDialog.descResult")}
          </DialogDescription>
        </DialogHeader>

        {step === "dropzone" && (
          <div className="space-y-4">
            <div
              role="button"
              tabIndex={0}
              className={`
                flex flex-col items-center justify-center
                border-2 border-dashed rounded-lg p-8
                transition-colors cursor-pointer
                ${isDragOver ? "border-primary bg-primary/5" : "border-muted-foreground/25 hover:border-muted-foreground/50"}
              `}
              onDrop={handleDrop}
              onDragOver={handleDragOver}
              onDragLeave={handleDragLeave}
              onClick={() =>
                document.getElementById("proxy-file-input")?.click()
              }
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  document.getElementById("proxy-file-input")?.click();
                }
              }}
            >
              <LuUpload className="w-10 h-10 text-muted-foreground mb-4" />
              <p className="text-sm text-muted-foreground text-center">
                {t("proxies.importDialog.dropzonePrompt")}
                <br />
                <span className="text-xs">
                  {t("proxies.importDialog.dropzoneFormats")}
                </span>
              </p>
              <input
                id="proxy-file-input"
                type="file"
                accept=".json,.txt"
                className="hidden"
                onChange={(e) => {
                  const file = e.target.files?.[0];
                  if (file) handleFileRead(file);
                  e.target.value = "";
                }}
              />
            </div>
            <p className="text-xs text-muted-foreground text-center">
              {t("proxies.importDialog.pasteHint", { modKey })}
            </p>
          </div>
        )}

        {step === "preview" && (
          <div className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="name-prefix">
                {t("proxies.importDialog.namePrefix")}
              </Label>
              <Input
                id="name-prefix"
                placeholder={t("proxies.importDialog.namePrefixDefault")}
                value={namePrefix}
                onChange={(e) => {
                  setNamePrefix(e.target.value);
                }}
              />
              <p className="text-xs text-muted-foreground">
                {t("proxies.importDialog.namePrefixHint", {
                  prefix:
                    namePrefix || t("proxies.importDialog.namePrefixDefault"),
                })}
              </p>
            </div>

            <div className="space-y-2">
              <Label>
                {t("proxies.importDialog.proxiesToImport", {
                  count: parsedProxies.length,
                })}
                {invalidProxies.length > 0 && (
                  <span className="text-muted-foreground ml-2">
                    {t("proxies.importDialog.invalidCount", {
                      count: invalidProxies.length,
                    })}
                  </span>
                )}
              </Label>
              <ScrollArea className="h-[200px] border rounded-md">
                <div className="p-2 space-y-1">
                  {parsedProxies.map((proxy, i) => (
                    <div
                      key={`${proxy.original_line}-${i}`}
                      className="text-xs font-mono p-2 bg-muted/30 rounded"
                    >
                      <span className="text-primary">
                        {proxy.proxy_type}://
                      </span>
                      {proxy.username && (
                        <span className="text-muted-foreground">
                          {proxy.username}:***@
                        </span>
                      )}
                      <span>
                        {proxy.host}:{proxy.port}
                      </span>
                    </div>
                  ))}
                </div>
              </ScrollArea>
            </div>
          </div>
        )}

        {step === "ambiguous" && (
          <div className="space-y-4">
            <p className="text-sm text-muted-foreground">
              {t("proxies.importDialog.ambiguousIntro")}
            </p>
            <ScrollArea className="h-[250px] border rounded-md">
              <div className="p-3 space-y-4">
                {ambiguousProxies.map((proxy, i) => (
                  <div
                    key={`${proxy.line}-${i}`}
                    className="space-y-2 pb-3 border-b last:border-0"
                  >
                    <code className="text-xs bg-muted px-2 py-1 rounded block">
                      {proxy.line}
                    </code>
                    <div className="flex flex-col gap-2">
                      {proxy.possible_formats.map((format) => (
                        <label
                          key={format}
                          className="flex items-center gap-2 cursor-pointer"
                        >
                          <input
                            type="radio"
                            name={`format-${i}`}
                            checked={proxy.selectedFormat === format}
                            onChange={() => {
                              handleAmbiguousFormatSelect(i, format);
                            }}
                            className="accent-primary"
                          />
                          <span className="text-xs">{format}</span>
                        </label>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            </ScrollArea>
          </div>
        )}

        {step === "result" && importResult && (
          <div className="space-y-4">
            <div className="p-4 bg-muted/30 rounded-lg space-y-2">
              <div className="flex justify-between">
                <span className="text-sm">
                  {t("proxies.importDialog.imported")}
                </span>
                <span className="text-sm font-medium text-success">
                  {importResult.imported_count}
                </span>
              </div>
              {importResult.skipped_count > 0 && (
                <div className="flex justify-between">
                  <span className="text-sm">
                    {t("proxies.importDialog.skippedDuplicates")}
                  </span>
                  <span className="text-sm font-medium text-warning">
                    {importResult.skipped_count}
                  </span>
                </div>
              )}
              {importResult.errors.length > 0 && (
                <div className="flex justify-between">
                  <span className="text-sm">
                    {t("proxies.importDialog.errors")}
                  </span>
                  <span className="text-sm font-medium text-destructive">
                    {importResult.errors.length}
                  </span>
                </div>
              )}
            </div>

            {importResult.errors.length > 0 && (
              <div className="space-y-2">
                <Label>{t("proxies.importDialog.errors")}</Label>
                <ScrollArea className="h-[100px] border rounded-md">
                  <div className="p-2 space-y-1">
                    {importResult.errors.map((error, i) => (
                      <div
                        key={`error-${i}`}
                        className="text-xs text-destructive"
                      >
                        {error}
                      </div>
                    ))}
                  </div>
                </ScrollArea>
              </div>
            )}
          </div>
        )}

        <DialogFooter>
          {step === "dropzone" && (
            <RippleButton variant="outline" onClick={handleClose}>
              {t("common.buttons.cancel")}
            </RippleButton>
          )}

          {step === "preview" && (
            <>
              <RippleButton variant="outline" onClick={resetState}>
                {t("common.buttons.back")}
              </RippleButton>
              <LoadingButton
                isLoading={isImporting}
                onClick={() => void handleImport()}
                disabled={parsedProxies.length === 0}
              >
                {t("proxies.importDialog.importButton", {
                  count: parsedProxies.length,
                })}
              </LoadingButton>
            </>
          )}

          {step === "ambiguous" && (
            <>
              <RippleButton variant="outline" onClick={resetState}>
                {t("common.buttons.back")}
              </RippleButton>
              <RippleButton
                onClick={handleResolveAmbiguous}
                disabled={ambiguousProxies.some((p) => !p.selectedFormat)}
              >
                {t("proxies.importDialog.continueButton")}
              </RippleButton>
            </>
          )}

          {step === "result" && (
            <RippleButton onClick={handleClose}>
              {t("proxies.importDialog.doneButton")}
            </RippleButton>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
