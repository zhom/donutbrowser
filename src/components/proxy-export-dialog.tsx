"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { LuCheck, LuCopy, LuDownload } from "react-icons/lu";
import { toast } from "sonner";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { ScrollArea } from "@/components/ui/scroll-area";
import { RippleButton } from "./ui/ripple";

interface ProxyExportDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function ProxyExportDialog({ isOpen, onClose }: ProxyExportDialogProps) {
  const [format, setFormat] = useState<"json" | "txt">("json");
  const [exportContent, setExportContent] = useState<string>("");
  const [isLoading, setIsLoading] = useState(false);
  const [copied, setCopied] = useState(false);

  const loadExportContent = useCallback(async () => {
    setIsLoading(true);
    try {
      const content = await invoke<string>("export_proxies", { format });
      setExportContent(content);
    } catch (error) {
      console.error("Failed to export proxies:", error);
      toast.error("Failed to export proxies");
      setExportContent("");
    } finally {
      setIsLoading(false);
    }
  }, [format]);

  useEffect(() => {
    if (isOpen) {
      void loadExportContent();
    }
  }, [isOpen, loadExportContent]);

  const handleCopyToClipboard = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(exportContent);
      setCopied(true);
      toast.success("Copied to clipboard");
      setTimeout(() => setCopied(false), 2000);
    } catch (error) {
      console.error("Failed to copy to clipboard:", error);
      toast.error("Failed to copy to clipboard");
    }
  }, [exportContent]);

  const handleDownload = useCallback(() => {
    const filename = format === "json" ? "proxies.json" : "proxies.txt";
    const mimeType = format === "json" ? "application/json" : "text/plain";

    const blob = new Blob([exportContent], { type: mimeType });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);

    toast.success(`Downloaded ${filename}`);
  }, [format, exportContent]);

  const handleClose = useCallback(() => {
    setFormat("json");
    setExportContent("");
    setCopied(false);
    onClose();
  }, [onClose]);

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Export Proxies</DialogTitle>
          <DialogDescription>
            Export your proxy configurations to a file
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label>Export Format</Label>
            <RadioGroup
              value={format}
              onValueChange={(value) => setFormat(value as "json" | "txt")}
              className="flex gap-4"
            >
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="json" id="format-json" />
                <Label htmlFor="format-json" className="cursor-pointer">
                  JSON
                </Label>
              </div>
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="txt" id="format-txt" />
                <Label htmlFor="format-txt" className="cursor-pointer">
                  TXT (URL format)
                </Label>
              </div>
            </RadioGroup>
          </div>

          <div className="space-y-2">
            <Label>Preview</Label>
            <ScrollArea className="h-[200px] border rounded-md bg-muted/30">
              {isLoading ? (
                <div className="flex items-center justify-center h-full p-4 text-sm text-muted-foreground">
                  Loading...
                </div>
              ) : exportContent ? (
                <pre className="p-3 text-xs font-mono whitespace-pre-wrap break-all">
                  {exportContent}
                </pre>
              ) : (
                <div className="flex items-center justify-center h-full p-4 text-sm text-muted-foreground">
                  No proxies to export
                </div>
              )}
            </ScrollArea>
          </div>
        </div>

        <DialogFooter className="flex-col sm:flex-row gap-2">
          <RippleButton variant="outline" onClick={handleClose}>
            Close
          </RippleButton>
          <RippleButton
            variant="outline"
            onClick={() => void handleCopyToClipboard()}
            disabled={!exportContent || isLoading}
            className="flex gap-2 items-center"
          >
            {copied ? (
              <LuCheck className="w-4 h-4" />
            ) : (
              <LuCopy className="w-4 h-4" />
            )}
            {copied ? "Copied" : "Copy"}
          </RippleButton>
          <RippleButton
            onClick={handleDownload}
            disabled={!exportContent || isLoading}
            className="flex gap-2 items-center"
          >
            <LuDownload className="w-4 h-4" />
            Download
          </RippleButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
