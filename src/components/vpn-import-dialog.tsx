"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuShield, LuUpload } from "react-icons/lu";
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
import { RippleButton } from "@/components/ui/ripple";
import { ScrollArea } from "@/components/ui/scroll-area";
import { getCurrentOS } from "@/lib/browser-utils";
import type { VpnImportResult, VpnType } from "@/types";

interface VpnImportDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

type ImportStep = "dropzone" | "vpn-preview" | "vpn-result";

interface VpnPreviewData {
  content: string;
  filename: string;
  detectedType: VpnType | null;
  endpoint: string | null;
}

const detectVpnType = (
  content: string,
  filename: string,
): { isVpn: boolean; type: VpnType | null; endpoint: string | null } => {
  const lowerFilename = filename.toLowerCase();
  if (
    lowerFilename.endsWith(".conf") &&
    content.includes("[Interface]") &&
    content.includes("[Peer]")
  ) {
    const endpointMatch = content.match(/Endpoint\s*=\s*([^\s\n]+)/i);
    return {
      isVpn: true,
      type: "WireGuard",
      endpoint: endpointMatch ? endpointMatch[1] : null,
    };
  }
  return { isVpn: false, type: null, endpoint: null };
};

export function VpnImportDialog({ isOpen, onClose }: VpnImportDialogProps) {
  const { t } = useTranslation();
  const [step, setStep] = useState<ImportStep>("dropzone");
  const [isDragOver, setIsDragOver] = useState(false);
  const [vpnPreview, setVpnPreview] = useState<VpnPreviewData | null>(null);
  const [vpnName, setVpnName] = useState("");
  const [vpnImportResult, setVpnImportResult] =
    useState<VpnImportResult | null>(null);
  const [isImporting, setIsImporting] = useState(false);

  const os = getCurrentOS();
  const modKey = os === "macos" ? "⌘" : "Ctrl";

  const resetState = useCallback(() => {
    setStep("dropzone");
    setIsDragOver(false);
    setVpnPreview(null);
    setVpnName("");
    setVpnImportResult(null);
    setIsImporting(false);
  }, []);

  const handleClose = useCallback(() => {
    resetState();
    onClose();
  }, [resetState, onClose]);

  const processContent = useCallback(
    (content: string, filename: string) => {
      const detection = detectVpnType(content, filename);
      if (!detection.isVpn) {
        toast.error(t("vpns.import.invalidContent"));
        return;
      }
      setVpnPreview({
        content,
        filename,
        detectedType: detection.type,
        endpoint: detection.endpoint,
      });
      const baseName = filename
        .replace(/\.conf$/i, "")
        .replace(/_/g, " ")
        .replace(/-/g, " ");
      setVpnName(baseName || `${detection.type} VPN`);
      setStep("vpn-preview");
    },
    [t],
  );

  const handleFileRead = useCallback(
    (file: File) => {
      const reader = new FileReader();
      reader.onload = (e) => {
        const content = e.target?.result as string;
        processContent(content, file.name);
      };
      reader.onerror = () => {
        toast.error(t("vpns.import.fileReadError"));
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
      const validFile = files.find((f) => f.name.endsWith(".conf"));
      if (validFile) {
        handleFileRead(validFile);
      } else {
        toast.error(t("vpns.import.wrongFileType"));
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

  useEffect(() => {
    if (!isOpen || step !== "dropzone") return;

    const handlePaste = (e: ClipboardEvent) => {
      const text = e.clipboardData?.getData("text");
      if (text) {
        processContent(text, "pasted.conf");
      }
    };

    document.addEventListener("paste", handlePaste);
    return () => {
      document.removeEventListener("paste", handlePaste);
    };
  }, [isOpen, step, processContent]);

  const handleImport = useCallback(async () => {
    if (!vpnPreview) return;
    setIsImporting(true);
    try {
      const result = await invoke<VpnImportResult>("import_vpn_config", {
        content: vpnPreview.content,
        filename: vpnPreview.filename,
        name: vpnName.trim() || null,
      });
      setVpnImportResult(result);
      setStep("vpn-result");
      if (result.success) {
        await emit("vpn-configs-changed");
      }
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : t("vpns.import.failedGeneric"),
      );
    } finally {
      setIsImporting(false);
    }
  }, [vpnPreview, vpnName, t]);

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("vpns.import.title")}</DialogTitle>
          <DialogDescription>
            {step === "dropzone" && t("vpns.import.descDropzone")}
            {step === "vpn-preview" && t("vpns.import.descPreview")}
            {step === "vpn-result" && t("vpns.import.descResult")}
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
              onClick={() => document.getElementById("vpn-file-input")?.click()}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  document.getElementById("vpn-file-input")?.click();
                }
              }}
            >
              <LuUpload className="w-10 h-10 text-muted-foreground mb-4" />
              <p className="text-sm text-muted-foreground text-center">
                {t("vpns.import.dropzonePrompt")}
              </p>
              <input
                id="vpn-file-input"
                type="file"
                accept=".conf"
                className="hidden"
                onChange={(e) => {
                  const file = e.target.files?.[0];
                  if (file) handleFileRead(file);
                  e.target.value = "";
                }}
              />
            </div>
            <p className="text-xs text-muted-foreground text-center">
              {t("vpns.import.pasteHint", { modKey })}
            </p>
          </div>
        )}

        {step === "vpn-preview" && vpnPreview && (
          <div className="space-y-4">
            <div className="flex items-center gap-3 p-4 bg-muted/30 rounded-lg">
              <LuShield className="w-8 h-8 text-primary" />
              <div>
                <div className="font-medium">
                  {t("vpns.import.configurationLabel", {
                    type: vpnPreview.detectedType,
                  })}
                </div>
                {vpnPreview.endpoint && (
                  <div className="text-sm text-muted-foreground">
                    {t("vpns.import.endpointLabel", {
                      endpoint: vpnPreview.endpoint,
                    })}
                  </div>
                )}
              </div>
            </div>

            <div className="space-y-2">
              <Label htmlFor="vpn-name">{t("vpns.import.vpnNameLabel")}</Label>
              <Input
                id="vpn-name"
                placeholder={t("vpns.import.vpnNamePlaceholder")}
                value={vpnName}
                onChange={(e) => {
                  setVpnName(e.target.value);
                }}
              />
            </div>

            <div className="space-y-2">
              <Label>{t("vpns.import.configPreview")}</Label>
              <ScrollArea className="h-[150px] border rounded-md">
                <pre className="p-2 text-xs font-mono whitespace-pre-wrap break-all">
                  {vpnPreview.content.slice(0, 1000)}
                  {vpnPreview.content.length > 1000 && "..."}
                </pre>
              </ScrollArea>
            </div>
          </div>
        )}

        {step === "vpn-result" && vpnImportResult && (
          <div className="space-y-4">
            <div
              className={`p-4 rounded-lg ${vpnImportResult.success ? "bg-success/10" : "bg-destructive/10"}`}
            >
              {vpnImportResult.success ? (
                <div className="flex items-center gap-3">
                  <LuShield className="w-8 h-8 text-success" />
                  <div>
                    <div className="font-medium text-success">
                      {t("vpns.import.importedSuccess")}
                    </div>
                    <div className="text-sm text-muted-foreground">
                      {vpnImportResult.name} ({vpnImportResult.vpn_type})
                    </div>
                  </div>
                </div>
              ) : (
                <div className="space-y-2">
                  <div className="font-medium text-destructive">
                    {t("vpns.import.importFailed")}
                  </div>
                  <div className="text-sm text-destructive">
                    {vpnImportResult.error}
                  </div>
                </div>
              )}
            </div>
          </div>
        )}

        <DialogFooter>
          {step === "dropzone" && (
            <RippleButton variant="outline" onClick={handleClose}>
              {t("common.buttons.cancel")}
            </RippleButton>
          )}

          {step === "vpn-preview" && (
            <>
              <RippleButton variant="outline" onClick={resetState}>
                {t("common.buttons.back")}
              </RippleButton>
              <LoadingButton
                isLoading={isImporting}
                onClick={() => void handleImport()}
              >
                {t("vpns.import.importButton")}
              </LoadingButton>
            </>
          )}

          {step === "vpn-result" && (
            <RippleButton onClick={handleClose}>
              {t("vpns.import.doneButton")}
            </RippleButton>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
