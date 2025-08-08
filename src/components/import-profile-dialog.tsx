"use client";

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";
import { FaFolder } from "react-icons/fa";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useBrowserSupport } from "@/hooks/use-browser-support";
import { getBrowserDisplayName, getBrowserIcon } from "@/lib/browser-utils";
import type { DetectedProfile } from "@/types";
import { RippleButton } from "./ui/ripple";

interface ImportProfileDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onImportComplete?: () => void;
}

export function ImportProfileDialog({
  isOpen,
  onClose,
  onImportComplete,
}: ImportProfileDialogProps) {
  const [detectedProfiles, setDetectedProfiles] = useState<DetectedProfile[]>(
    [],
  );
  const [isLoading, setIsLoading] = useState(false);
  const [isImporting, setIsImporting] = useState(false);
  const [importMode, setImportMode] = useState<"auto-detect" | "manual">(
    "auto-detect",
  );

  // Auto-detect state
  const [selectedDetectedProfile, setSelectedDetectedProfile] = useState<
    string | null
  >(null);
  const [autoDetectProfileName, setAutoDetectProfileName] = useState("");

  // Manual import state
  const [manualBrowserType, setManualBrowserType] = useState<string | null>(
    null,
  );
  const [manualProfilePath, setManualProfilePath] = useState("");
  const [manualProfileName, setManualProfileName] = useState("");

  const { supportedBrowsers, isLoading: isLoadingSupport } =
    useBrowserSupport();

  const loadDetectedProfiles = useCallback(async () => {
    setIsLoading(true);
    try {
      const profiles = await invoke<DetectedProfile[]>(
        "detect_existing_profiles",
      );
      setDetectedProfiles(profiles);

      // Auto-switch to manual mode if no profiles detected
      if (profiles.length === 0) {
        setImportMode("manual");
      } else {
        // Auto-select first profile if available
        setSelectedDetectedProfile(profiles[0].path);

        // Generate default name from the detected profile
        const profile = profiles[0];
        const browserName = getBrowserDisplayName(profile.browser);
        const defaultName = `Imported ${browserName} Profile`;
        setAutoDetectProfileName(defaultName);
      }
    } catch (error) {
      console.error("Failed to detect existing profiles:", error);
      toast.error("Failed to detect existing browser profiles");
    } finally {
      setIsLoading(false);
    }
  }, []);

  const handleBrowseFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Select Browser Profile Folder",
      });

      if (selected && typeof selected === "string") {
        setManualProfilePath(selected);
      }
    } catch (error) {
      console.error("Failed to open folder dialog:", error);
      toast.error("Failed to open folder dialog");
    }
  };

  const handleAutoDetectImport = useCallback(async () => {
    if (!selectedDetectedProfile || !autoDetectProfileName.trim()) {
      toast.error("Please select a profile and provide a name");
      return;
    }

    const profile = detectedProfiles.find(
      (p) => p.path === selectedDetectedProfile,
    );
    if (!profile) {
      toast.error("Selected profile not found");
      return;
    }

    setIsImporting(true);
    try {
      await invoke("import_browser_profile", {
        sourcePath: profile.path,
        browserType: profile.browser,
        newProfileName: autoDetectProfileName.trim(),
      });

      toast.success(
        `Successfully imported profile "${autoDetectProfileName.trim()}"`,
      );
      if (onImportComplete) {
        onImportComplete();
      }
      onClose();
    } catch (error) {
      console.error("Failed to import profile:", error);
      const errorMessage =
        error instanceof Error ? error.message : String(error);

      // Check if error is about browser not being downloaded
      if (errorMessage.includes("No downloaded versions found")) {
        const browserDisplayName = getBrowserDisplayName(profile.browser);
        toast.error(
          `${browserDisplayName} is not installed. Please download ${browserDisplayName} first from the main window, then try importing again.`,
          {
            duration: 8000,
          },
        );
      } else {
        toast.error(`Failed to import profile: ${errorMessage}`);
      }
    } finally {
      setIsImporting(false);
    }
  }, [
    selectedDetectedProfile,
    autoDetectProfileName,
    detectedProfiles,
    onImportComplete,
    onClose,
  ]);

  const handleManualImport = useCallback(async () => {
    if (
      !manualBrowserType ||
      !manualProfilePath.trim() ||
      !manualProfileName.trim()
    ) {
      toast.error("Please fill in all fields");
      return;
    }

    setIsImporting(true);
    try {
      await invoke("import_browser_profile", {
        sourcePath: manualProfilePath.trim(),
        browserType: manualBrowserType,
        newProfileName: manualProfileName.trim(),
      });

      toast.success(
        `Successfully imported profile "${manualProfileName.trim()}"`,
      );
      if (onImportComplete) {
        onImportComplete();
      }
      onClose();
    } catch (error) {
      console.error("Failed to import profile:", error);
      const errorMessage =
        error instanceof Error ? error.message : String(error);

      // Check if error is about browser not being downloaded
      if (errorMessage.includes("No downloaded versions found")) {
        const browserDisplayName = getBrowserDisplayName(manualBrowserType);
        toast.error(
          `${browserDisplayName} is not installed. Please download ${browserDisplayName} first from the main window, then try importing again.`,
          {
            duration: 8000,
          },
        );
      } else {
        toast.error(`Failed to import profile: ${errorMessage}`);
      }
    } finally {
      setIsImporting(false);
    }
  }, [
    manualBrowserType,
    manualProfilePath,
    manualProfileName,
    onImportComplete,
    onClose,
  ]);

  const handleClose = () => {
    setSelectedDetectedProfile(null);
    setAutoDetectProfileName("");
    setManualBrowserType(null);
    setManualProfilePath("");
    setManualProfileName("");
    // Only reset to auto-detect if there are profiles available
    if (detectedProfiles.length > 0) {
      setImportMode("auto-detect");
    } else {
      setImportMode("manual");
    }
    onClose();
  };

  // Update auto-detect profile name when selection changes
  useEffect(() => {
    if (selectedDetectedProfile) {
      const profile = detectedProfiles.find(
        (p) => p.path === selectedDetectedProfile,
      );
      if (profile) {
        const browserName = getBrowserDisplayName(profile.browser);
        const defaultName = `Old ${browserName}`;
        setAutoDetectProfileName(defaultName);
      }
    }
  }, [selectedDetectedProfile, detectedProfiles]);

  const selectedProfile = detectedProfiles.find(
    (p) => p.path === selectedDetectedProfile,
  );

  useEffect(() => {
    if (isOpen) {
      void loadDetectedProfiles();
    }
  }, [isOpen, loadDetectedProfiles]);

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-2xl max-h-[80vh] my-8 flex flex-col">
        <DialogHeader className="flex-shrink-0">
          <DialogTitle>Import Browser Profile</DialogTitle>
        </DialogHeader>

        <div className="overflow-y-auto flex-1 space-y-6 min-h-0">
          {/* Mode Selection */}
          <div className="flex gap-2">
            <RippleButton
              variant={importMode === "auto-detect" ? "default" : "outline"}
              onClick={() => {
                setImportMode("auto-detect");
              }}
              className="flex-1"
              disabled={isLoading}
            >
              Auto-Detect
            </RippleButton>
            <RippleButton
              variant={importMode === "manual" ? "default" : "outline"}
              onClick={() => {
                setImportMode("manual");
              }}
              className="flex-1"
              disabled={isLoading}
            >
              Manual Import
            </RippleButton>
          </div>

          {/* Auto-Detect Mode */}
          {importMode === "auto-detect" && (
            <div className="space-y-4">
              <h3 className="text-lg font-medium">Detected Browser Profiles</h3>

              {isLoading ? (
                <div className="py-8 text-center">
                  <p className="text-muted-foreground">
                    Scanning for browser profiles...
                  </p>
                </div>
              ) : detectedProfiles.length === 0 ? (
                <div className="py-8 text-center">
                  <p className="text-muted-foreground">
                    No browser profiles found on your system.
                  </p>
                  <p className="mt-2 text-sm text-muted-foreground">
                    Try the manual import option if you have profiles in custom
                    locations.
                  </p>
                </div>
              ) : (
                <div className="space-y-4">
                  <div>
                    <Label htmlFor="detected-profile-select" className="mb-2">
                      Select Profile:
                    </Label>
                    <Select
                      value={selectedDetectedProfile ?? undefined}
                      onValueChange={(value) => {
                        setSelectedDetectedProfile(value);
                      }}
                    >
                      <SelectTrigger id="detected-profile-select">
                        <SelectValue placeholder="Choose a detected profile" />
                      </SelectTrigger>
                      <SelectContent>
                        {detectedProfiles.map((profile) => {
                          const IconComponent = getBrowserIcon(profile.browser);
                          return (
                            <SelectItem key={profile.path} value={profile.path}>
                              <div className="flex gap-2 items-center">
                                {IconComponent && (
                                  <IconComponent className="w-4 h-4" />
                                )}
                                <div className="flex flex-col">
                                  <span className="font-medium">
                                    {profile.name}
                                  </span>
                                  <span className="text-xs text-muted-foreground">
                                    {profile.description}
                                  </span>
                                </div>
                              </div>
                            </SelectItem>
                          );
                        })}
                      </SelectContent>
                    </Select>
                  </div>

                  {selectedProfile && (
                    <div className="p-3 rounded-lg bg-muted">
                      <p className="text-sm">
                        <span className="font-medium">Path:</span>{" "}
                        {selectedProfile.path}
                      </p>
                      <p className="text-sm">
                        <span className="font-medium">Browser:</span>{" "}
                        {getBrowserDisplayName(selectedProfile.browser)}
                      </p>
                    </div>
                  )}

                  <div>
                    <Label htmlFor="auto-profile-name" className="mb-2">
                      New Profile Name:
                    </Label>
                    <Input
                      id="auto-profile-name"
                      value={autoDetectProfileName}
                      onChange={(e) => {
                        setAutoDetectProfileName(e.target.value);
                      }}
                      placeholder="Enter a name for the imported profile"
                    />
                  </div>
                </div>
              )}
            </div>
          )}

          {/* Manual Import Mode */}
          {importMode === "manual" && (
            <div className="space-y-4">
              <h3 className="text-lg font-medium">Manual Profile Import</h3>

              <div className="space-y-4">
                <div>
                  <Label htmlFor="manual-browser-select" className="mb-2">
                    Browser Type:
                  </Label>
                  <Select
                    value={manualBrowserType ?? undefined}
                    onValueChange={(value) => {
                      setManualBrowserType(value);
                    }}
                    disabled={isLoadingSupport}
                  >
                    <SelectTrigger id="manual-browser-select">
                      <SelectValue
                        placeholder={
                          isLoadingSupport
                            ? "Loading browsers..."
                            : "Select browser type"
                        }
                      />
                    </SelectTrigger>
                    <SelectContent>
                      {supportedBrowsers.map((browser) => {
                        const IconComponent = getBrowserIcon(browser);
                        return (
                          <SelectItem key={browser} value={browser}>
                            <div className="flex gap-2 items-center">
                              {IconComponent && (
                                <IconComponent className="w-4 h-4" />
                              )}
                              <span>{getBrowserDisplayName(browser)}</span>
                            </div>
                          </SelectItem>
                        );
                      })}
                    </SelectContent>
                  </Select>
                </div>

                <div>
                  <Label htmlFor="manual-profile-path" className="mb-2">
                    Profile Folder Path:
                  </Label>
                  <div className="flex gap-2">
                    <Input
                      id="manual-profile-path"
                      value={manualProfilePath}
                      onChange={(e) => {
                        setManualProfilePath(e.target.value);
                      }}
                      placeholder="Enter the full path to the profile folder"
                    />
                    <Button
                      variant="outline"
                      size="icon"
                      onClick={() => void handleBrowseFolder()}
                      title="Browse for folder"
                    >
                      <FaFolder className="w-4 h-4" />
                    </Button>
                  </div>
                  <p className="mt-2 text-xs text-muted-foreground">
                    Example paths:
                    <br />
                    macOS: ~/Library/Application
                    Support/Firefox/Profiles/xxx.default
                    <br />
                    Windows: %APPDATA%\Mozilla\Firefox\Profiles\xxx.default
                    <br />
                    Linux: ~/.mozilla/firefox/xxx.default
                  </p>
                </div>

                <div>
                  <Label htmlFor="manual-profile-name" className="mb-2">
                    New Profile Name:
                  </Label>
                  <Input
                    id="manual-profile-name"
                    value={manualProfileName}
                    onChange={(e) => {
                      setManualProfileName(e.target.value);
                    }}
                    placeholder="Enter a name for the imported profile"
                  />
                </div>
              </div>
            </div>
          )}
        </div>

        <DialogFooter className="flex-shrink-0">
          <RippleButton variant="outline" onClick={handleClose}>
            Cancel
          </RippleButton>
          {importMode === "auto-detect" ? (
            <LoadingButton
              isLoading={isImporting}
              onClick={() => {
                void handleAutoDetectImport();
              }}
              disabled={
                !selectedDetectedProfile ||
                !autoDetectProfileName.trim() ||
                isLoading
              }
            >
              Import
            </LoadingButton>
          ) : (
            <LoadingButton
              isLoading={isImporting}
              onClick={() => {
                void handleManualImport();
              }}
              disabled={
                !manualBrowserType ||
                !manualProfilePath.trim() ||
                !manualProfileName.trim()
              }
            >
              Import
            </LoadingButton>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
