"use client";

import { LoadingButton } from "@/components/loading-button";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { VersionSelector } from "@/components/version-selector";
import { useBrowserDownload } from "@/hooks/use-browser-download";
import type { BrowserProfile } from "@/types";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { LuTriangleAlert } from "react-icons/lu";

interface ChangeVersionDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
  onVersionChanged: () => void;
}

export function ChangeVersionDialog({
  isOpen,
  onClose,
  profile,
  onVersionChanged,
}: ChangeVersionDialogProps) {
  const [selectedVersion, setSelectedVersion] = useState<string | null>(null);
  const [isUpdating, setIsUpdating] = useState(false);
  const [showDowngradeWarning, setShowDowngradeWarning] = useState(false);
  const [acknowledgeDowngrade, setAcknowledgeDowngrade] = useState(false);

  const {
    availableVersions,
    downloadedVersions,
    isDownloading,
    loadVersions,
    loadDownloadedVersions,
    downloadBrowser,
    isVersionDownloaded,
  } = useBrowserDownload();

  useEffect(() => {
    if (isOpen && profile) {
      setSelectedVersion(profile.version);
      setAcknowledgeDowngrade(false);
      void loadVersions(profile.browser);
      void loadDownloadedVersions(profile.browser);
    }
  }, [isOpen, profile, loadVersions, loadDownloadedVersions]);

  useEffect(() => {
    if (profile && selectedVersion) {
      // Check if this is a downgrade
      const currentVersionIndex = availableVersions.findIndex(
        (v) => v.tag_name === profile.version,
      );
      const selectedVersionIndex = availableVersions.findIndex(
        (v) => v.tag_name === selectedVersion,
      );

      // If selected version has a higher index, it's older (downgrade)
      const isDowngrade =
        currentVersionIndex !== -1 &&
        selectedVersionIndex !== -1 &&
        selectedVersionIndex > currentVersionIndex;
      setShowDowngradeWarning(isDowngrade);

      if (!isDowngrade) {
        setAcknowledgeDowngrade(false);
      }
    }
  }, [selectedVersion, profile, availableVersions]);

  const handleDownload = async () => {
    if (!profile || !selectedVersion) return;
    await downloadBrowser(profile.browser, selectedVersion);
  };

  const handleVersionChange = async () => {
    if (!profile || !selectedVersion) return;

    setIsUpdating(true);
    try {
      await invoke("update_profile_version", {
        profileName: profile.name,
        version: selectedVersion,
      });
      onVersionChanged();
      onClose();
    } catch (error) {
      console.error("Failed to update profile version:", error);
    } finally {
      setIsUpdating(false);
    }
  };

  const canUpdate =
    profile &&
    selectedVersion &&
    selectedVersion !== profile.version &&
    selectedVersion &&
    isVersionDownloaded(selectedVersion) &&
    (!showDowngradeWarning || acknowledgeDowngrade);

  if (!profile) return null;

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Change Browser Version</DialogTitle>
        </DialogHeader>

        <div className="grid gap-4 py-4">
          <div className="space-y-2">
            <Label className="text-sm font-medium">Profile:</Label>
            <div className="p-2 bg-muted rounded text-sm">{profile.name}</div>
          </div>

          <div className="space-y-2">
            <Label className="text-sm font-medium">Current Version:</Label>
            <div className="p-2 bg-muted rounded text-sm">
              {profile.version}
            </div>
          </div>

          {/* Version Selection */}
          <div className="grid gap-2">
            <Label>New Version</Label>
            <VersionSelector
              selectedVersion={selectedVersion}
              onVersionSelect={setSelectedVersion}
              availableVersions={availableVersions}
              downloadedVersions={downloadedVersions}
              isDownloading={isDownloading}
              onDownload={() => {
                void handleDownload();
              }}
              placeholder="Select version..."
            />
          </div>

          {/* Downgrade Warning */}
          {showDowngradeWarning && (
            <Alert className="border-orange-700">
              <LuTriangleAlert className="h-4 w-4 text-orange-700" />
              <AlertTitle className="text-orange-700">
                Downgrade Warning
              </AlertTitle>
              <AlertDescription className="text-orange-700">
                You are about to downgrade from version {profile.version} to{" "}
                {selectedVersion}. This may lead to compatibility issues, data
                loss, or unexpected behavior.
                <div className="flex items-center space-x-2 mt-3">
                  <Checkbox
                    id="acknowledge-downgrade"
                    checked={acknowledgeDowngrade}
                    onCheckedChange={(checked) => {
                      setAcknowledgeDowngrade(checked as boolean);
                    }}
                  />
                  <Label htmlFor="acknowledge-downgrade" className="text-sm">
                    I understand the risks and want to proceed
                  </Label>
                </div>
              </AlertDescription>
            </Alert>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <LoadingButton
            isLoading={isUpdating}
            onClick={() => {
              void handleVersionChange();
            }}
            disabled={!canUpdate}
          >
            {isUpdating ? "Updating..." : "Update Version"}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
