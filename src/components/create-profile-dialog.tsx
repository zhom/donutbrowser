"use client";

import { LoadingButton } from "@/components/loading-button";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
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
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { VersionSelector } from "@/components/version-selector";
import { useBrowserDownload } from "@/hooks/use-browser-download";
import { useBrowserSupport } from "@/hooks/use-browser-support";
import { getBrowserDisplayName } from "@/lib/browser-utils";
import type { BrowserProfile, ProxySettings } from "@/types";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { toast } from "sonner";

type BrowserTypeString =
  | "mullvad-browser"
  | "firefox"
  | "firefox-developer"
  | "chromium"
  | "brave"
  | "zen"
  | "tor-browser";

interface CreateProfileDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onCreateProfile: (profileData: {
    name: string;
    browserStr: BrowserTypeString;
    version: string;
    proxy?: ProxySettings;
  }) => Promise<void>;
}

export function CreateProfileDialog({
  isOpen,
  onClose,
  onCreateProfile,
}: CreateProfileDialogProps) {
  const [profileName, setProfileName] = useState("");
  const [selectedBrowser, setSelectedBrowser] =
    useState<BrowserTypeString | null>("mullvad-browser");
  const [selectedVersion, setSelectedVersion] = useState<string | null>(null);
  const [isCreating, setIsCreating] = useState(false);
  const [existingProfiles, setExistingProfiles] = useState<BrowserProfile[]>(
    [],
  );

  // Proxy settings
  const [proxyEnabled, setProxyEnabled] = useState(false);
  const [proxyType, setProxyType] = useState("http");
  const [proxyHost, setProxyHost] = useState("");
  const [proxyPort, setProxyPort] = useState(8080);

  const {
    availableVersions,
    downloadedVersions,
    isDownloading,
    loadVersions,
    loadDownloadedVersions,
    downloadBrowser,
    isVersionDownloaded,
  } = useBrowserDownload();

  const {
    supportedBrowsers,
    isLoading: isLoadingSupport,
    isBrowserSupported,
  } = useBrowserSupport();

  useEffect(() => {
    if (isOpen) {
      void loadExistingProfiles();
    }
  }, [isOpen]);

  useEffect(() => {
    if (supportedBrowsers.length > 0) {
      // Set default browser to first supported browser
      if (supportedBrowsers.includes("mullvad-browser")) {
        setSelectedBrowser("mullvad-browser");
      } else if (supportedBrowsers.length > 0) {
        setSelectedBrowser(supportedBrowsers[0] as BrowserTypeString);
      }
    }
  }, [supportedBrowsers]);

  useEffect(() => {
    if (isOpen && selectedBrowser) {
      // Reset selected version when browser changes
      setSelectedVersion(null);
      void loadVersions(selectedBrowser);
      void loadDownloadedVersions(selectedBrowser);
    }
  }, [isOpen, selectedBrowser, loadVersions, loadDownloadedVersions]);

  // Set default version when versions are loaded and no version is selected
  useEffect(() => {
    if (availableVersions.length > 0 && selectedBrowser) {
      // Always reset version when browser changes or versions are loaded
      // Find the latest stable version (not alpha/beta)
      const stableVersions = availableVersions.filter((v) => !v.is_nightly);

      if (stableVersions.length > 0) {
        // Select the first stable version (they're already sorted newest first)
        setSelectedVersion(stableVersions[0].tag_name);
      } else if (availableVersions.length > 0) {
        // If no stable version found, select the first available version
        setSelectedVersion(availableVersions[0].tag_name);
      }
    }
  }, [availableVersions, selectedBrowser]);

  const loadExistingProfiles = async () => {
    try {
      const profiles = await invoke<BrowserProfile[]>("list_browser_profiles");
      setExistingProfiles(profiles);
    } catch (error) {
      console.error("Failed to load existing profiles:", error);
    }
  };

  const handleDownload = async () => {
    if (!selectedBrowser || !selectedVersion) return;
    await downloadBrowser(selectedBrowser, selectedVersion);
  };

  const validateProfileName = (name: string): string | null => {
    const trimmedName = name.trim();

    if (!trimmedName) {
      return "Profile name cannot be empty";
    }

    // Check for duplicate names (case insensitive)
    const isDuplicate = existingProfiles.some(
      (profile) => profile.name.toLowerCase() === trimmedName.toLowerCase(),
    );

    if (isDuplicate) {
      return "A profile with this name already exists";
    }

    return null;
  };

  // Helper to determine if proxy should be disabled for the selected browser
  const isProxyDisabled = selectedBrowser === "tor-browser";

  // Update proxy enabled state when browser changes to tor-browser
  useEffect(() => {
    if (selectedBrowser === "tor-browser" && proxyEnabled) {
      setProxyEnabled(false);
    }
  }, [selectedBrowser, proxyEnabled]);

  const handleCreate = async () => {
    if (!profileName.trim() || !selectedBrowser || !selectedVersion) return;

    // Validate profile name
    const nameError = validateProfileName(profileName);
    if (nameError) {
      toast.error(nameError);
      return;
    }

    setIsCreating(true);
    try {
      const proxy =
        proxyEnabled && !isProxyDisabled
          ? {
              enabled: true,
              proxy_type: proxyType,
              host: proxyHost,
              port: proxyPort,
            }
          : undefined;

      await onCreateProfile({
        name: profileName.trim(),
        browserStr: selectedBrowser,
        version: selectedVersion,
        proxy,
      });

      // Reset form
      setProfileName("");
      setSelectedVersion(null);
      setProxyEnabled(false);
      setProxyHost("");
      setProxyPort(8080);
      onClose();
    } catch (error) {
      console.error("Failed to create profile:", error);
    } finally {
      setIsCreating(false);
    }
  };

  const nameError = profileName.trim()
    ? validateProfileName(profileName)
    : null;
  const canCreate =
    profileName.trim() &&
    selectedBrowser &&
    selectedVersion &&
    isVersionDownloaded(selectedVersion) &&
    (!proxyEnabled || isProxyDisabled || (proxyHost && proxyPort)) &&
    !nameError;

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Create New Profile</DialogTitle>
        </DialogHeader>

        <div className="grid gap-4 py-4">
          {/* Profile Name */}
          <div className="grid gap-2">
            <Label htmlFor="profile-name">Profile Name</Label>
            <Input
              id="profile-name"
              value={profileName}
              onChange={(e) => {
                setProfileName(e.target.value);
              }}
              placeholder="Enter profile name"
              className={nameError ? "border-red-500" : ""}
            />
            {nameError && <p className="text-sm text-red-600">{nameError}</p>}
          </div>

          {/* Browser Selection */}
          <div className="grid gap-2">
            <Label>Browser</Label>
            <Select
              value={selectedBrowser ?? undefined}
              onValueChange={(value) => {
                setSelectedBrowser(value as BrowserTypeString);
              }}
              disabled={isLoadingSupport}
            >
              <SelectTrigger>
                <SelectValue
                  placeholder={
                    isLoadingSupport ? "Loading browsers..." : "Select browser"
                  }
                />
              </SelectTrigger>
              <SelectContent>
                {(
                  [
                    "mullvad-browser",
                    "firefox",
                    "firefox-developer",
                    "chromium",
                    "brave",
                    "zen",
                    "tor-browser",
                  ] as BrowserTypeString[]
                ).map((browser) => {
                  const isSupported = isBrowserSupported(browser);
                  const displayName = getBrowserDisplayName(browser);

                  if (!isSupported) {
                    return (
                      <Tooltip key={browser}>
                        <TooltipTrigger asChild>
                          <SelectItem
                            value={browser}
                            disabled={true}
                            className="opacity-50"
                          >
                            {displayName} (Not supported on this platform)
                          </SelectItem>
                        </TooltipTrigger>
                        <TooltipContent>
                          <p>
                            {displayName} is not supported on your current
                            platform or architecture.
                          </p>
                        </TooltipContent>
                      </Tooltip>
                    );
                  }

                  return (
                    <SelectItem key={browser} value={browser}>
                      {displayName}
                    </SelectItem>
                  );
                })}
              </SelectContent>
            </Select>
          </div>

          {/* Version Selection */}
          <div className="grid gap-2">
            <Label>Version</Label>
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

          {/* Proxy Settings */}
          <div className="grid gap-4 pt-4 border-t">
            <div className="flex items-center space-x-2">
              {isProxyDisabled ? (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <div className="flex items-center space-x-2 opacity-50">
                      <Checkbox
                        id="proxy-enabled"
                        checked={false}
                        disabled={true}
                      />
                      <Label htmlFor="proxy-enabled" className="text-gray-500">
                        Enable Proxy
                      </Label>
                    </div>
                  </TooltipTrigger>
                  <TooltipContent>
                    <p>
                      Tor Browser has its own built-in proxy system and
                      doesn&apos;t support additional proxy configuration
                    </p>
                  </TooltipContent>
                </Tooltip>
              ) : (
                <>
                  <Checkbox
                    id="proxy-enabled"
                    checked={proxyEnabled}
                    onCheckedChange={(checked) => {
                      setProxyEnabled(checked as boolean);
                    }}
                  />
                  <Label htmlFor="proxy-enabled">Enable Proxy</Label>
                </>
              )}
            </div>

            {proxyEnabled && !isProxyDisabled && (
              <>
                <div className="grid gap-2">
                  <Label>Proxy Type</Label>
                  <Select value={proxyType} onValueChange={setProxyType}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {["http", "https", "socks4", "socks5"].map((type) => (
                        <SelectItem key={type} value={type}>
                          {type.toUpperCase()}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="proxy-host">Host</Label>
                  <Input
                    id="proxy-host"
                    value={proxyHost}
                    onChange={(e) => {
                      setProxyHost(e.target.value);
                    }}
                    placeholder="e.g. 127.0.0.1"
                  />
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="proxy-port">Port</Label>
                  <Input
                    id="proxy-port"
                    type="number"
                    value={proxyPort}
                    onChange={(e) => {
                      setProxyPort(Number.parseInt(e.target.value, 10) || 0);
                    }}
                    placeholder="e.g. 8080"
                    min="1"
                    max="65535"
                  />
                </div>
              </>
            )}
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <LoadingButton
            isLoading={isCreating}
            onClick={() => void handleCreate()}
            disabled={!canCreate}
          >
            Create Profile
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
