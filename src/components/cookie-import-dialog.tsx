"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useState } from "react";
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
import { RippleButton } from "@/components/ui/ripple";
import type { BrowserProfile } from "@/types";

interface CookieImportResult {
  cookies_imported: number;
  cookies_replaced: number;
  errors: string[];
}

interface CookieImportDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
}

const countCookies = (content: string): number => {
  const trimmed = content.trim();
  if (trimmed.startsWith("[")) {
    try {
      const arr = JSON.parse(trimmed);
      if (Array.isArray(arr)) return arr.length;
    } catch {
      // Fall through to Netscape counting
    }
  }
  return content.split("\n").filter((line) => {
    const l = line.trim();
    return l && !l.startsWith("#");
  }).length;
};

export function CookieImportDialog({
  isOpen,
  onClose,
  profile,
}: CookieImportDialogProps) {
  const [fileContent, setFileContent] = useState<string | null>(null);
  const [fileName, setFileName] = useState<string | null>(null);
  const [cookieCount, setCookieCount] = useState(0);
  const [isImporting, setIsImporting] = useState(false);
  const [result, setResult] = useState<CookieImportResult | null>(null);

  const resetState = useCallback(() => {
    setFileContent(null);
    setFileName(null);
    setCookieCount(0);
    setIsImporting(false);
    setResult(null);
  }, []);

  const handleClose = useCallback(() => {
    resetState();
    onClose();
  }, [resetState, onClose]);

  const handleFileRead = useCallback((file: File) => {
    const reader = new FileReader();
    reader.onload = (e) => {
      const content = e.target?.result as string;
      setFileContent(content);
      setFileName(file.name);
      setCookieCount(countCookies(content));
    };
    reader.onerror = () => {
      toast.error("Failed to read file");
    };
    reader.readAsText(file);
  }, []);

  const handleImport = useCallback(async () => {
    if (!fileContent || !profile) return;
    setIsImporting(true);
    try {
      const importResult = await invoke<CookieImportResult>(
        "import_cookies_from_file",
        {
          profileId: profile.id,
          content: fileContent,
        },
      );
      setResult(importResult);
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
    } finally {
      setIsImporting(false);
    }
  }, [fileContent, profile]);

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Import Cookies</DialogTitle>
          <DialogDescription>
            {!fileContent &&
              "Import cookies from a Netscape or JSON format file."}
            {fileContent &&
              !result &&
              `${cookieCount} cookies found in ${fileName}`}
            {result && "Cookie import completed"}
          </DialogDescription>
        </DialogHeader>

        {!fileContent && (
          <div className="space-y-4">
            <div
              role="button"
              tabIndex={0}
              className="flex flex-col items-center justify-center border-2 border-dashed rounded-lg p-8 transition-colors cursor-pointer border-muted-foreground/25 hover:border-muted-foreground/50"
              onClick={() =>
                document.getElementById("cookie-file-input")?.click()
              }
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  document.getElementById("cookie-file-input")?.click();
                }
              }}
            >
              <LuUpload className="w-10 h-10 text-muted-foreground mb-4" />
              <p className="text-sm text-muted-foreground text-center">
                Click to choose a cookie file
                <br />
                <span className="text-xs">(.txt, .cookies, or .json)</span>
              </p>
              <input
                id="cookie-file-input"
                type="file"
                accept=".txt,.cookies,.json"
                className="hidden"
                onChange={(e) => {
                  const file = e.target.files?.[0];
                  if (file) handleFileRead(file);
                  e.target.value = "";
                }}
              />
            </div>
          </div>
        )}

        {fileContent && !result && (
          <div className="space-y-4">
            <div className="flex items-center gap-3 p-4 bg-muted/30 rounded-lg">
              <div>
                <div className="font-medium">{fileName}</div>
                <div className="text-sm text-muted-foreground">
                  {cookieCount} cookies found
                </div>
              </div>
            </div>
          </div>
        )}

        {result && (
          <div className="space-y-4">
            <div className="p-4 rounded-lg bg-green-500/10">
              <div className="font-medium text-green-600 dark:text-green-400">
                Successfully imported {result.cookies_imported} cookies (
                {result.cookies_replaced} replaced)
              </div>
              {result.errors.length > 0 && (
                <div className="mt-2 text-sm text-muted-foreground">
                  {result.errors.length} line(s) skipped
                </div>
              )}
            </div>
          </div>
        )}

        <DialogFooter>
          {!fileContent && (
            <RippleButton variant="outline" onClick={handleClose}>
              Cancel
            </RippleButton>
          )}

          {fileContent && !result && (
            <>
              <RippleButton variant="outline" onClick={resetState}>
                Back
              </RippleButton>
              <LoadingButton
                isLoading={isImporting}
                onClick={() => void handleImport()}
                disabled={cookieCount === 0}
              >
                Import
              </LoadingButton>
            </>
          )}

          {result && <RippleButton onClick={handleClose}>Done</RippleButton>}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
