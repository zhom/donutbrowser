"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { LoadingButton } from "@/components/loading-button";
import { ReleaseTypeSelector } from "@/components/release-type-selector";
import { Alert, AlertDescription } from "@/components/ui/alert";
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
  // Nightly switching is disabled for non-nightly profiles (except Firefox Developer),
  // so downgrade warnings are no longer applicable.

  const {
    downloadedVersions,
    isBrowserDownloading,
    loadDownloadedVersions,
    downloadBrowser,
    isVersionDownloaded,
  } = useBrowserDownload();

  const loadReleaseTypes = useCallback(
    async (browser: string) => {
      setIsLoadingReleaseTypes(true);
      try {
        const releaseTypes = await invoke<BrowserReleaseTypes>(
          "get_browser_release_types",
          { browserStr: browser },
        );
        // Filter nightly visibility based on rules:
        // - Firefox Developer Edition: allow nightly only
        // - If profile is currently nightly: allow both stable and nightly
        // - Otherwise: allow stable only
        const filtered: BrowserReleaseTypes = {};
        if (profile?.browser === "firefox-developer") {
          if (releaseTypes.nightly) filtered.nightly = releaseTypes.nightly;
        } else if (profile?.release_type === "nightly") {
          if (releaseTypes.stable) filtered.stable = releaseTypes.stable;
          if (releaseTypes.nightly) filtered.nightly = releaseTypes.nightly;
        } else {
          if (releaseTypes.stable) filtered.stable = releaseTypes.stable;
        }
        setReleaseTypes(filtered);
      } catch (error) {
        console.error("Failed to load release types:", error);
      } finally {
        setIsLoadingReleaseTypes(false);
      }
    },
    [profile?.browser, profile?.release_type],
  );

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
    isVersionDownloaded(selectedVersion);

  useEffect(() => {
    if (isOpen && profile) {
      // Set current release type based on profile
      setSelectedReleaseType(profile.release_type as "stable" | "nightly");
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

          {/* Nightly switching disabled in UI; no downgrade warning needed. */}
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
