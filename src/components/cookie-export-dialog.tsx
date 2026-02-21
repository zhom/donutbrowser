"use client";

import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "@tauri-apps/plugin-fs";
import { useCallback, useState } from "react";
import { LuDownload } from "react-icons/lu";
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
import { Label } from "@/components/ui/label";
import { RippleButton } from "@/components/ui/ripple";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { BrowserProfile } from "@/types";

interface CookieExportDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
}

export function CookieExportDialog({
  isOpen,
  onClose,
  profile,
}: CookieExportDialogProps) {
  const [format, setFormat] = useState<"netscape" | "json">("json");
  const [isExporting, setIsExporting] = useState(false);

  const handleClose = useCallback(() => {
    setFormat("json");
    setIsExporting(false);
    onClose();
  }, [onClose]);

  const handleExport = useCallback(async () => {
    if (!profile) return;
    setIsExporting(true);
    try {
      const content = await invoke<string>("export_profile_cookies", {
        profileId: profile.id,
        format,
      });

      const ext = format === "json" ? "json" : "txt";
      const defaultName = `${profile.name}_cookies.${ext}`;

      const filePath = await save({
        defaultPath: defaultName,
        filters: [
          {
            name: format === "json" ? "JSON" : "Text",
            extensions: [ext],
          },
        ],
      });

      if (!filePath) {
        setIsExporting(false);
        return;
      }

      await writeTextFile(filePath, content);
      toast.success("Cookies exported successfully");
      handleClose();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
    } finally {
      setIsExporting(false);
    }
  }, [profile, format, handleClose]);

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Export Cookies</DialogTitle>
          <DialogDescription>
            Export cookies from this profile.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label>Format</Label>
            <Select
              value={format}
              onValueChange={(v) => setFormat(v as "netscape" | "json")}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="json">JSON</SelectItem>
                <SelectItem value="netscape">Netscape TXT</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>

        <DialogFooter>
          <RippleButton variant="outline" onClick={handleClose}>
            Cancel
          </RippleButton>
          <LoadingButton
            isLoading={isExporting}
            onClick={() => void handleExport()}
          >
            <LuDownload className="w-4 h-4 mr-2" />
            Export
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
