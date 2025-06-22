"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import { ReleaseTypeSelector } from "@/components/release-type-selector";
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
import { useBrowserDownload } from "@/hooks/use-browser-download";
import { useBrowserSupport } from "@/hooks/use-browser-support";
import { getBrowserDisplayName } from "@/lib/browser-utils";
import type {
  BrowserProfile,
  BrowserReleaseTypes,
  ProxySettings,
} from "@/types";
import { Alert, AlertDescription } from "./ui/alert";

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
    releaseType: string;
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
  const [selectedReleaseType, setSelectedReleaseType] = useState<
    "stable" | "nightly" | null
  >(null);
  const [releaseTypes, setReleaseTypes] = useState<BrowserReleaseTypes>({
    stable: undefined,
    nightly: undefined,
  });
  const [isCreating, setIsCreating] = useState(false);
  const [existingProfiles, setExistingProfiles] = useState<BrowserProfile[]>(
    [],
  );
  const [isLoadingReleaseTypes, setIsLoadingReleaseTypes] = useState(false);

  // Proxy settings
  const [proxyEnabled, setProxyEnabled] = useState(false);
  const [proxyType, setProxyType] = useState("http");
  const [proxyHost, setProxyHost] = useState("");
  const [proxyPort, setProxyPort] = useState(8080);
  const [proxyUsername, setProxyUsername] = useState("");
  const [proxyPassword, setProxyPassword] = useState("");

  const {
    downloadBrowser,
    isDownloading,
    downloadedVersions,
    loadDownloadedVersions,
    isVersionDownloaded,
  } = useBrowserDownload();

  const {
    supportedBrowsers,
    isLoading: isLoadingSupport,
    isBrowserSupported,
  } = useBrowserSupport();

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

  // Set default release type when release types are loaded
  useEffect(() => {
    if (!selectedReleaseType && Object.keys(releaseTypes).length > 0) {
      // First try to set stable if it exists
      if (releaseTypes.stable) {
        setSelectedReleaseType("stable");
      }
      // If stable doesn't exist but nightly does, set nightly as default
      else if (releaseTypes.nightly && selectedBrowser !== "chromium") {
        setSelectedReleaseType("nightly");
      }
    }
  }, [releaseTypes, selectedReleaseType, selectedBrowser]);

  const loadExistingProfiles = useCallback(async () => {
    try {
      const profiles = await invoke<BrowserProfile[]>("list_browser_profiles");
      setExistingProfiles(profiles);
    } catch (error) {
      console.error("Failed to load existing profiles:", error);
    }
  }, []);

  const loadReleaseTypes = useCallback(async (browser: string) => {
    try {
      setIsLoadingReleaseTypes(true);
      const types = await invoke<BrowserReleaseTypes>(
        "get_browser_release_types",
        {
          browserStr: browser,
        },
      );
      setReleaseTypes(types);
    } catch (error) {
      console.error("Failed to load release types:", error);
      toast.error("Failed to load available versions");
    } finally {
      setIsLoadingReleaseTypes(false);
    }
  }, []);

  const handleDownload = useCallback(async () => {
    if (!selectedBrowser || !selectedReleaseType) return;

    const version =
      selectedReleaseType === "stable"
        ? releaseTypes.stable
        : releaseTypes.nightly;
    if (!version) return;

    await downloadBrowser(selectedBrowser, version);
  }, [selectedBrowser, selectedReleaseType, downloadBrowser, releaseTypes]);

  const validateProfileName = useCallback(
    (name: string): string | null => {
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
    },
    [existingProfiles],
  );

  // Helper to determine if proxy should be disabled for the selected browser
  const isProxyDisabled = selectedBrowser === "tor-browser";

  // Update proxy enabled state when browser changes to tor-browser
  useEffect(() => {
    if (selectedBrowser === "tor-browser" && proxyEnabled) {
      setProxyEnabled(false);
    }
  }, [selectedBrowser, proxyEnabled]);

  const handleCreate = useCallback(async () => {
    if (!profileName.trim() || !selectedBrowser || !selectedReleaseType) return;

    // Validate profile name
    const nameError = validateProfileName(profileName);
    if (nameError) {
      toast.error(nameError);
      return;
    }

    const version =
      selectedReleaseType === "stable"
        ? releaseTypes.stable
        : releaseTypes.nightly;
    if (!version) {
      toast.error("Selected release type is not available");
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
              username: proxyUsername || undefined,
              password: proxyPassword || undefined,
            }
          : undefined;

      await onCreateProfile({
        name: profileName.trim(),
        browserStr: selectedBrowser,
        version,
        releaseType: selectedReleaseType,
        proxy,
      });

      // Reset form
      setProfileName("");
      setSelectedReleaseType(null);
      setProxyEnabled(false);
      setProxyHost("");
      setProxyPort(8080);
      setProxyUsername("");
      setProxyPassword("");
      onClose();
    } catch (error) {
      console.error("Failed to create profile:", error);
    } finally {
      setIsCreating(false);
    }
  }, [
    profileName,
    selectedBrowser,
    selectedReleaseType,
    onCreateProfile,
    proxyEnabled,
    isProxyDisabled,
    onClose,
    proxyHost,
    proxyPassword,
    proxyPort,
    proxyType,
    proxyUsername,
    releaseTypes.nightly,
    releaseTypes.stable,
    validateProfileName,
  ]);

  const nameError = profileName.trim()
    ? validateProfileName(profileName)
    : null;

  const selectedVersion =
    selectedReleaseType === "stable"
      ? releaseTypes.stable
      : releaseTypes.nightly;

  const canCreate =
    profileName.trim() &&
    selectedBrowser &&
    selectedReleaseType &&
    selectedVersion &&
    isVersionDownloaded(selectedVersion) &&
    (!proxyEnabled || isProxyDisabled || (proxyHost && proxyPort)) &&
    !nameError;

  useEffect(() => {
    if (isOpen) {
      void loadExistingProfiles();
    }
  }, [isOpen, loadExistingProfiles]);

  useEffect(() => {
    if (isOpen && selectedBrowser) {
      // Reset selected release type when browser changes
      setSelectedReleaseType(null);
      void loadReleaseTypes(selectedBrowser);
      void loadDownloadedVersions(selectedBrowser);
    }
  }, [isOpen, selectedBrowser, loadDownloadedVersions, loadReleaseTypes]);

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md max-h-[80vh] my-8 flex flex-col">
        <DialogHeader className="flex-shrink-0">
          <DialogTitle>Create New Profile</DialogTitle>
        </DialogHeader>

        <div className="grid overflow-y-scroll flex-1 gap-6 py-4 min-h-0">
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
                            {displayName} (Not supported)
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

          {selectedBrowser ? (
            <div className="grid gap-2">
              <Label>Release Type</Label>
              {isLoadingReleaseTypes ? (
                <div className="text-sm text-muted-foreground">
                  Loading release types...
                </div>
              ) : Object.keys(releaseTypes).length === 0 ? (
                <Alert>
                  <AlertDescription>
                    No releases are available for{" "}
                    {getBrowserDisplayName(selectedBrowser)}.
                  </AlertDescription>
                </Alert>
              ) : (
                <div className="space-y-4">
                  {(!releaseTypes.stable || !releaseTypes.nightly) && (
                    <Alert>
                      <AlertDescription>
                        Only {(releaseTypes.stable && "Stable") ?? "Nightly"}{" "}
                        releases are available for{" "}
                        {getBrowserDisplayName(selectedBrowser)}.
                      </AlertDescription>
                    </Alert>
                  )}

                  <ReleaseTypeSelector
                    selectedReleaseType={selectedReleaseType}
                    onReleaseTypeSelect={setSelectedReleaseType}
                    availableReleaseTypes={releaseTypes}
                    browser={selectedBrowser}
                    isDownloading={isDownloading}
                    onDownload={() => {
                      void handleDownload();
                    }}
                    placeholder="Select release type..."
                    downloadedVersions={downloadedVersions}
                  />
                </div>
              )}
            </div>
          ) : null}

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

                <div className="grid gap-2">
                  <Label htmlFor="proxy-username">Username (optional)</Label>
                  <Input
                    id="proxy-username"
                    value={proxyUsername}
                    onChange={(e) => {
                      setProxyUsername(e.target.value);
                    }}
                    placeholder="Proxy username"
                  />
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="proxy-password">Password (optional)</Label>
                  <Input
                    id="proxy-password"
                    type="password"
                    value={proxyPassword}
                    onChange={(e) => {
                      setProxyPassword(e.target.value);
                    }}
                    placeholder="Proxy password"
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
