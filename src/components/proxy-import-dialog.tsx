"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
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
import { ScrollArea } from "@/components/ui/scroll-area";
import { getCurrentOS } from "@/lib/browser-utils";
import type {
  ParsedProxyLine,
  ProxyImportResult,
  ProxyParseResult,
  VpnImportResult,
  VpnType,
} from "@/types";
import { RippleButton } from "./ui/ripple";

interface ProxyImportDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

type ImportStep =
  | "dropzone"
  | "preview"
  | "ambiguous"
  | "result"
  | "vpn-preview"
  | "vpn-result";

interface AmbiguousProxy {
  line: string;
  possible_formats: string[];
  selectedFormat?: string;
}

interface VpnPreviewData {
  content: string;
  filename: string;
  detectedType: VpnType | null;
  endpoint: string | null;
}

export function ProxyImportDialog({ isOpen, onClose }: ProxyImportDialogProps) {
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
  const [namePrefix, setNamePrefix] = useState("Imported");
  // VPN import state
  const [vpnPreview, setVpnPreview] = useState<VpnPreviewData | null>(null);
  const [vpnName, setVpnName] = useState("");
  const [vpnImportResult, setVpnImportResult] =
    useState<VpnImportResult | null>(null);

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
    setNamePrefix("Imported");
    // Reset VPN state
    setVpnPreview(null);
    setVpnName("");
    setVpnImportResult(null);
  }, []);

  // Detect VPN type from content
  const detectVpnType = useCallback(
    (
      content: string,
      filename: string,
    ): { isVpn: boolean; type: VpnType | null; endpoint: string | null } => {
      const lowerFilename = filename.toLowerCase();

      // Check for WireGuard config
      if (
        lowerFilename.endsWith(".conf") &&
        content.includes("[Interface]") &&
        content.includes("[Peer]")
      ) {
        // Extract endpoint from WireGuard config
        const endpointMatch = content.match(/Endpoint\s*=\s*([^\s\n]+)/i);
        return {
          isVpn: true,
          type: "WireGuard",
          endpoint: endpointMatch ? endpointMatch[1] : null,
        };
      }

      // Check for OpenVPN config
      if (
        lowerFilename.endsWith(".ovpn") ||
        (content.includes("remote ") &&
          (content.includes("client") || content.includes("dev tun")))
      ) {
        // Extract remote from OpenVPN config
        const remoteMatch = content.match(/remote\s+(\S+)(?:\s+(\d+))?/i);
        const endpoint = remoteMatch
          ? `${remoteMatch[1]}${remoteMatch[2] ? `:${remoteMatch[2]}` : ""}`
          : null;
        return { isVpn: true, type: "OpenVPN", endpoint };
      }

      return { isVpn: false, type: null, endpoint: null };
    },
    [],
  );

  const processContent = useCallback(
    async (content: string, isJson: boolean, filename: string = "") => {
      try {
        // Check if it's a VPN config
        const vpnDetection = detectVpnType(content, filename);
        if (vpnDetection.isVpn) {
          setVpnPreview({
            content,
            filename,
            detectedType: vpnDetection.type,
            endpoint: vpnDetection.endpoint,
          });
          // Generate default name from filename
          const baseName = filename
            .replace(/\.(conf|ovpn)$/i, "")
            .replace(/_/g, " ")
            .replace(/-/g, " ");
          setVpnName(baseName || `${vpnDetection.type} VPN`);
          setStep("vpn-preview");
          return;
        }

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
            toast.error("No valid proxies found in the file");
          }
        }
      } catch (error) {
        console.error("Failed to process content:", error);
        toast.error(
          error instanceof Error ? error.message : "Failed to process file",
        );
      } finally {
        setIsImporting(false);
      }
    },
    [detectVpnType],
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
        toast.error("Failed to read file");
      };
      reader.readAsText(file);
    },
    [processContent],
  );

  const handleDrop = useCallback(
    (e: React.DragEvent<HTMLDivElement>) => {
      e.preventDefault();
      setIsDragOver(false);

      const files = Array.from(e.dataTransfer.files);
      const validFile = files.find(
        (f) =>
          f.name.endsWith(".json") ||
          f.name.endsWith(".txt") ||
          f.name.endsWith(".conf") || // WireGuard
          f.name.endsWith(".ovpn"), // OpenVPN
      );

      if (validFile) {
        handleFileRead(validFile);
      } else {
        toast.error("Please drop a .json, .txt, .conf, or .ovpn file");
      }
    },
    [handleFileRead],
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

    const handlePaste = async (e: ClipboardEvent) => {
      const text = e.clipboardData?.getData("text");
      if (text) {
        // Try to detect if it's JSON
        const trimmed = text.trim();
        const isJson =
          (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
          (trimmed.startsWith("[") && trimmed.endsWith("]"));
        // Use "pasted.txt" as filename to trigger content-based detection
        await processContent(text, isJson, "pasted.txt");
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
          namePrefix: namePrefix.trim() || "Imported",
        },
      );
      setImportResult(result);
      setStep("result");
      await emit("stored-proxies-changed");
    } catch (error) {
      console.error("Failed to import proxies:", error);
      toast.error(
        error instanceof Error ? error.message : "Failed to import proxies",
      );
    } finally {
      setIsImporting(false);
    }
  }, [parsedProxies, namePrefix]);

  const handleVpnImport = useCallback(async () => {
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
      console.error("Failed to import VPN config:", error);
      toast.error(
        error instanceof Error ? error.message : "Failed to import VPN config",
      );
    } finally {
      setIsImporting(false);
    }
  }, [vpnPreview, vpnName]);

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
          <DialogTitle>
            {step === "vpn-preview" || step === "vpn-result"
              ? "Import VPN Config"
              : "Import Proxies"}
          </DialogTitle>
          <DialogDescription>
            {step === "dropzone" &&
              "Import proxies from a JSON or TXT file, or VPN configs (.conf/.ovpn)"}
            {step === "preview" && "Review the proxies to import"}
            {step === "ambiguous" &&
              "Some proxies have ambiguous formats. Please select the correct format."}
            {step === "result" && "Import completed"}
            {step === "vpn-preview" && "Review the VPN configuration to import"}
            {step === "vpn-result" && "VPN import completed"}
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
                Drop a proxy or VPN config file
                <br />
                <span className="text-xs">(.json, .txt, .conf, .ovpn)</span>
              </p>
              <input
                id="proxy-file-input"
                type="file"
                accept=".json,.txt,.conf,.ovpn"
                className="hidden"
                onChange={(e) => {
                  const file = e.target.files?.[0];
                  if (file) handleFileRead(file);
                  e.target.value = "";
                }}
              />
            </div>
            <p className="text-xs text-muted-foreground text-center">
              Paste from clipboard with {modKey}+V
            </p>
          </div>
        )}

        {step === "preview" && (
          <div className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="name-prefix">Name Prefix</Label>
              <Input
                id="name-prefix"
                placeholder="Imported"
                value={namePrefix}
                onChange={(e) => setNamePrefix(e.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                Proxies will be named &quot;{namePrefix || "Imported"} Proxy
                1&quot;, &quot;{namePrefix || "Imported"} Proxy 2&quot;, etc.
              </p>
            </div>

            <div className="space-y-2">
              <Label>
                Proxies to import ({parsedProxies.length})
                {invalidProxies.length > 0 && (
                  <span className="text-muted-foreground ml-2">
                    ({invalidProxies.length} invalid)
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
              The following proxies have an ambiguous format. Please select the
              correct interpretation for each.
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
                            onChange={() =>
                              handleAmbiguousFormatSelect(i, format)
                            }
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
                <span className="text-sm">Imported:</span>
                <span className="text-sm font-medium text-green-600 dark:text-green-400">
                  {importResult.imported_count}
                </span>
              </div>
              {importResult.skipped_count > 0 && (
                <div className="flex justify-between">
                  <span className="text-sm">Skipped (duplicates):</span>
                  <span className="text-sm font-medium text-yellow-600 dark:text-yellow-400">
                    {importResult.skipped_count}
                  </span>
                </div>
              )}
              {importResult.errors.length > 0 && (
                <div className="flex justify-between">
                  <span className="text-sm">Errors:</span>
                  <span className="text-sm font-medium text-red-600 dark:text-red-400">
                    {importResult.errors.length}
                  </span>
                </div>
              )}
            </div>

            {importResult.errors.length > 0 && (
              <div className="space-y-2">
                <Label>Errors</Label>
                <ScrollArea className="h-[100px] border rounded-md">
                  <div className="p-2 space-y-1">
                    {importResult.errors.map((error, i) => (
                      <div
                        key={`error-${i}`}
                        className="text-xs text-red-600 dark:text-red-400"
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

        {step === "vpn-preview" && vpnPreview && (
          <div className="space-y-4">
            <div className="flex items-center gap-3 p-4 bg-muted/30 rounded-lg">
              <LuShield className="w-8 h-8 text-primary" />
              <div>
                <div className="font-medium">
                  {vpnPreview.detectedType} Configuration
                </div>
                {vpnPreview.endpoint && (
                  <div className="text-sm text-muted-foreground">
                    Endpoint: {vpnPreview.endpoint}
                  </div>
                )}
              </div>
            </div>

            <div className="space-y-2">
              <Label htmlFor="vpn-name">VPN Name</Label>
              <Input
                id="vpn-name"
                placeholder="My VPN"
                value={vpnName}
                onChange={(e) => setVpnName(e.target.value)}
              />
            </div>

            <div className="space-y-2">
              <Label>Config Preview</Label>
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
              className={`p-4 rounded-lg ${vpnImportResult.success ? "bg-green-500/10" : "bg-red-500/10"}`}
            >
              {vpnImportResult.success ? (
                <div className="flex items-center gap-3">
                  <LuShield className="w-8 h-8 text-green-600 dark:text-green-400" />
                  <div>
                    <div className="font-medium text-green-600 dark:text-green-400">
                      VPN Imported Successfully
                    </div>
                    <div className="text-sm text-muted-foreground">
                      {vpnImportResult.name} ({vpnImportResult.vpn_type})
                    </div>
                  </div>
                </div>
              ) : (
                <div className="space-y-2">
                  <div className="font-medium text-red-600 dark:text-red-400">
                    Import Failed
                  </div>
                  <div className="text-sm text-red-600 dark:text-red-400">
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
              Cancel
            </RippleButton>
          )}

          {step === "preview" && (
            <>
              <RippleButton variant="outline" onClick={resetState}>
                Back
              </RippleButton>
              <LoadingButton
                isLoading={isImporting}
                onClick={() => void handleImport()}
                disabled={parsedProxies.length === 0}
              >
                Import {parsedProxies.length} Proxies
              </LoadingButton>
            </>
          )}

          {step === "ambiguous" && (
            <>
              <RippleButton variant="outline" onClick={resetState}>
                Back
              </RippleButton>
              <RippleButton
                onClick={handleResolveAmbiguous}
                disabled={ambiguousProxies.some((p) => !p.selectedFormat)}
              >
                Continue
              </RippleButton>
            </>
          )}

          {step === "result" && (
            <RippleButton onClick={handleClose}>Done</RippleButton>
          )}

          {step === "vpn-preview" && (
            <>
              <RippleButton variant="outline" onClick={resetState}>
                Back
              </RippleButton>
              <LoadingButton
                isLoading={isImporting}
                onClick={() => void handleVpnImport()}
              >
                Import VPN
              </LoadingButton>
            </>
          )}

          {step === "vpn-result" && (
            <RippleButton onClick={handleClose}>Done</RippleButton>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
