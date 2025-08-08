"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { LuTriangleAlert } from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import { ReleaseTypeSelector } from "@/components/release-type-selector";
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
import { useBrowserDownload } from "@/hooks/use-browser-download";
import { getBrowserDisplayName } from "@/lib/browser-utils";
import type { BrowserProfile, BrowserReleaseTypes } from "@/types";
import { RippleButton } from "./ui/ripple";

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
  const [selectedReleaseType, setSelectedReleaseType] = useState<
    "stable" | "nightly" | null
  >(null);
  const [releaseTypes, setReleaseTypes] = useState<BrowserReleaseTypes>({});
  const [isLoadingReleaseTypes, setIsLoadingReleaseTypes] = useState(false);
  const [isUpdating, setIsUpdating] = useState(false);
  const [showDowngradeWarning, setShowDowngradeWarning] = useState(false);
  const [acknowledgeDowngrade, setAcknowledgeDowngrade] = useState(false);

  const {
    downloadedVersions,
    isBrowserDownloading,
    loadDownloadedVersions,
    downloadBrowser,
    isVersionDownloaded,
  } = useBrowserDownload();

  const loadReleaseTypes = useCallback(async (browser: string) => {
    setIsLoadingReleaseTypes(true);
    try {
      const releaseTypes = await invoke<BrowserReleaseTypes>(
        "get_browser_release_types",
        { browserStr: browser },
      );
      setReleaseTypes(releaseTypes);
    } catch (error) {
      console.error("Failed to load release types:", error);
    } finally {
      setIsLoadingReleaseTypes(false);
    }
  }, []);

  useEffect(() => {
    if (
      profile &&
      selectedReleaseType &&
      selectedReleaseType !== profile.release_type
    ) {
      // For simplicity, we'll show downgrade warning when switching from stable to nightly
      // since nightly versions might be considered "downgrades" in terms of stability
      const isDowngrade =
        profile.release_type === "stable" && selectedReleaseType === "nightly";
      setShowDowngradeWarning(isDowngrade);

      if (!isDowngrade) {
        setAcknowledgeDowngrade(false);
      }
    }
  }, [selectedReleaseType, profile]);

  const handleDownload = useCallback(async () => {
    if (!profile || !selectedReleaseType) return;

    const version =
      selectedReleaseType === "stable"
        ? releaseTypes.stable
        : releaseTypes.nightly;
    if (!version) return;

    await downloadBrowser(profile.browser, version);
  }, [profile, selectedReleaseType, downloadBrowser, releaseTypes]);

  const handleVersionChange = useCallback(async () => {
    if (!profile || !selectedReleaseType) return;

    const version =
      selectedReleaseType === "stable"
        ? releaseTypes.stable
        : releaseTypes.nightly;
    if (!version) return;

    setIsUpdating(true);
    try {
      await invoke("update_profile_version", {
        profileName: profile.name,
        version,
      });
      onVersionChanged();
      onClose();
    } catch (error) {
      console.error("Failed to update profile version:", error);
    } finally {
      setIsUpdating(false);
    }
  }, [profile, selectedReleaseType, releaseTypes, onVersionChanged, onClose]);

  const selectedVersion =
    selectedReleaseType === "stable"
      ? releaseTypes.stable
      : releaseTypes.nightly;

  const canUpdate =
    profile &&
    selectedReleaseType &&
    selectedReleaseType !== profile.release_type &&
    selectedVersion &&
    isVersionDownloaded(selectedVersion) &&
    (!showDowngradeWarning || acknowledgeDowngrade);

  useEffect(() => {
    if (isOpen && profile) {
      // Set current release type based on profile
      setSelectedReleaseType(profile.release_type as "stable" | "nightly");
      setAcknowledgeDowngrade(false);
      void loadReleaseTypes(profile.browser);
      void loadDownloadedVersions(profile.browser);
    }
  }, [isOpen, profile, loadDownloadedVersions, loadReleaseTypes]);

  if (!profile) return null;

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Change Release Type</DialogTitle>
        </DialogHeader>

        <div className="grid gap-4 py-4">
          <div className="space-y-2">
            <Label className="text-sm font-medium">Profile:</Label>
            <div className="p-2 text-sm rounded bg-muted">{profile.name}</div>
          </div>

          <div className="space-y-2">
            <Label className="text-sm font-medium">Current Release:</Label>
            <div className="p-2 text-sm capitalize rounded bg-muted">
              {profile.release_type} ({profile.version})
            </div>
          </div>

          {!releaseTypes.stable && !releaseTypes.nightly ? (
            <Alert>
              <AlertDescription>
                No releases are available for{" "}
                {getBrowserDisplayName(profile.browser)}.
              </AlertDescription>
            </Alert>
          ) : !releaseTypes.stable || !releaseTypes.nightly ? (
            <div className="space-y-4">
              <Alert>
                <AlertDescription>
                  Only {profile.release_type} releases are available for{" "}
                  {getBrowserDisplayName(profile.browser)}.
                </AlertDescription>
              </Alert>
              <div className="grid gap-2">
                <Label>New Release Type</Label>
                {isLoadingReleaseTypes ? (
                  <div className="text-sm text-muted-foreground">
                    Loading release types...
                  </div>
                ) : (
                  <div className="space-y-4">
                    {selectedReleaseType &&
                      selectedReleaseType !== profile.release_type &&
                      selectedVersion &&
                      !isVersionDownloaded(selectedVersion) && (
                        <Alert>
                          <AlertDescription>
                            You must download{" "}
                            {getBrowserDisplayName(profile.browser)}{" "}
                            {selectedVersion} before switching to this release
                            type. Use the download button above to get the
                            latest version.
                          </AlertDescription>
                        </Alert>
                      )}

                    <ReleaseTypeSelector
                      selectedReleaseType={selectedReleaseType}
                      onReleaseTypeSelect={setSelectedReleaseType}
                      availableReleaseTypes={releaseTypes}
                      browser={profile.browser}
                      isDownloading={isBrowserDownloading(profile.browser)}
                      onDownload={() => {
                        void handleDownload();
                      }}
                      placeholder="Select release type..."
                      downloadedVersions={downloadedVersions}
                    />
                  </div>
                )}
              </div>
            </div>
          ) : (
            <div className="grid gap-2">
              <Label>New Release Type</Label>
              {isLoadingReleaseTypes ? (
                <div className="text-sm text-muted-foreground">
                  Loading release types...
                </div>
              ) : (
                <div className="space-y-4">
                  {selectedReleaseType &&
                    selectedReleaseType !== profile.release_type &&
                    selectedVersion &&
                    !isVersionDownloaded(selectedVersion) && (
                      <Alert>
                        <AlertDescription>
                          You must download{" "}
                          {getBrowserDisplayName(profile.browser)}{" "}
                          {selectedVersion} before switching to this release
                          type. Use the download button above to get the latest
                          version.
                        </AlertDescription>
                      </Alert>
                    )}

                  <ReleaseTypeSelector
                    selectedReleaseType={selectedReleaseType}
                    onReleaseTypeSelect={setSelectedReleaseType}
                    availableReleaseTypes={releaseTypes}
                    browser={profile.browser}
                    isDownloading={isBrowserDownloading(profile.browser)}
                    onDownload={() => {
                      void handleDownload();
                    }}
                    placeholder="Select release type..."
                    downloadedVersions={downloadedVersions}
                  />
                </div>
              )}
            </div>
          )}

          {/* Downgrade Warning */}
          {showDowngradeWarning && (
            <Alert className="border-orange-700">
              <LuTriangleAlert className="w-4 h-4 text-orange-700" />
              <AlertTitle className="text-orange-700">
                Stability Warning
              </AlertTitle>
              <AlertDescription className="text-orange-700">
                You are about to switch from stable to nightly releases. Nightly
                versions may be less stable and could contain bugs or incomplete
                features.
                <div className="flex items-center mt-3 space-x-2">
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
          <RippleButton variant="outline" onClick={onClose}>
            Cancel
          </RippleButton>
          <LoadingButton
            isLoading={isUpdating}
            onClick={() => {
              void handleVersionChange();
            }}
            disabled={!canUpdate}
          >
            {isUpdating ? "Updating..." : "Update Release Type"}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
