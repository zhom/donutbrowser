"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { LuCopy } from "react-icons/lu";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
import { useBrowserState } from "@/hooks/use-browser-support";
import { getBrowserDisplayName, getBrowserIcon } from "@/lib/browser-utils";
import type { BrowserProfile, StoredProxy } from "@/types";

interface ProfileSelectorDialogProps {
  isOpen: boolean;
  onClose: () => void;
  isUpdating: (browser: string) => boolean;
  url?: string;
  runningProfiles?: Set<string>;
}

export function ProfileSelectorDialog({
  isOpen,
  onClose,
  url,
  runningProfiles = new Set(),
  isUpdating,
}: ProfileSelectorDialogProps) {
  const [profiles, setProfiles] = useState<BrowserProfile[]>([]);
  const [selectedProfile, setSelectedProfile] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isLaunching, setIsLaunching] = useState(false);
  const [storedProxies, setStoredProxies] = useState<StoredProxy[]>([]);

  // Use shared browser state hook
  const browserState = useBrowserState(profiles, runningProfiles, isUpdating);

  // Helper function to check if a profile has a proxy
  const hasProxy = useCallback(
    (profile: BrowserProfile): boolean => {
      if (!profile.proxy_id) return false;
      const proxy = storedProxies.find((p) => p.id === profile.proxy_id);
      return proxy !== undefined;
    },
    [storedProxies],
  );

  const loadProfiles = useCallback(async () => {
    setIsLoading(true);
    try {
      // Load both profiles and stored proxies
      const [profileList, proxiesList] = await Promise.all([
        invoke<BrowserProfile[]>("list_browser_profiles"),
        invoke<StoredProxy[]>("get_stored_proxies"),
      ]);

      // Sort profiles by name
      profileList.sort((a, b) => a.name.localeCompare(b.name));

      // Set both profiles and proxies
      setProfiles(profileList);
      setStoredProxies(proxiesList);

      // Auto-select first available profile for link opening
      if (profileList.length > 0) {
        // First, try to find a running profile that can be used for opening links
        const runningAvailableProfile = profileList.find((profile) => {
          const isRunning = runningProfiles.has(profile.name);
          // Simple check without browserState dependency
          return (
            isRunning &&
            profile.browser !== "tor-browser" &&
            profile.browser !== "mullvad-browser"
          );
        });

        if (runningAvailableProfile) {
          setSelectedProfile(runningAvailableProfile.name);
        } else {
          // If no running profile is available, find the first available profile
          const availableProfile = profileList.find(
            (profile) =>
              profile.browser !== "tor-browser" &&
              profile.browser !== "mullvad-browser",
          );
          if (availableProfile) {
            setSelectedProfile(availableProfile.name);
          }
        }
      }
    } catch (err) {
      console.error("Failed to load profiles:", err);
    } finally {
      setIsLoading(false);
    }
  }, [runningProfiles]);

  // Helper function to get tooltip content for profiles - now uses shared hook
  const getProfileTooltipContent = (profile: BrowserProfile): string | null => {
    return browserState.getProfileTooltipContent(profile);
  };

  const handleOpenUrl = useCallback(async () => {
    if (!selectedProfile || !url) return;

    setIsLaunching(true);
    try {
      await invoke("open_url_with_profile", {
        profileName: selectedProfile,
        url,
      });
      onClose();
    } catch (error) {
      console.error("Failed to open URL with profile:", error);
    } finally {
      setIsLaunching(false);
    }
  }, [selectedProfile, url, onClose]);

  const handleCancel = useCallback(() => {
    setSelectedProfile(null);
    onClose();
  }, [onClose]);

  const handleCopyUrl = useCallback(async () => {
    if (!url) return;

    try {
      await navigator.clipboard.writeText(url);
      toast.success("URL copied to clipboard!");
    } catch (error) {
      console.error("Failed to copy URL:", error);
      toast.error("Failed to copy URL to clipboard");
    }
  }, [url]);

  const selectedProfileData = profiles.find((p) => p.name === selectedProfile);

  // Check if the selected profile can be used for opening links
  const canOpenWithSelectedProfile = () => {
    if (!selectedProfileData) return false;
    return browserState.canUseProfileForLinks(selectedProfileData);
  };

  // Get tooltip content for disabled profiles
  const getTooltipContent = () => {
    if (!selectedProfileData) return null;
    return getProfileTooltipContent(selectedProfileData);
  };

  useEffect(() => {
    if (isOpen) {
      void loadProfiles();
    }
  }, [isOpen, loadProfiles]);

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Choose Profile</DialogTitle>
        </DialogHeader>

        <div className="grid gap-4 py-4">
          {url && (
            <div className="space-y-2">
              <div className="flex justify-between items-center">
                <Label className="text-sm font-medium">Opening URL:</Label>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => void handleCopyUrl()}
                  className="flex gap-2 items-center"
                >
                  <LuCopy className="w-3 h-3" />
                  Copy
                </Button>
              </div>
              <div className="p-2 text-sm break-all rounded bg-muted">
                {url}
              </div>
            </div>
          )}

          <div className="space-y-2">
            <Label htmlFor="profile-select">Select Profile:</Label>
            {isLoading ? (
              <div className="text-sm text-muted-foreground">
                Loading profiles...
              </div>
            ) : profiles.length === 0 ? (
              <div className="space-y-2">
                <div className="text-sm text-muted-foreground">
                  No profiles available. Please create a profile first.
                </div>
                <div className="text-xs text-muted-foreground">
                  Close this dialog and create a profile from the main window to
                  get started.
                </div>
              </div>
            ) : (
              <Select
                value={selectedProfile ?? undefined}
                onValueChange={setSelectedProfile}
              >
                <SelectTrigger>
                  <SelectValue placeholder="Choose a profile" />
                </SelectTrigger>
                <SelectContent>
                  {profiles.map((profile) => {
                    const isRunning = runningProfiles.has(profile.name);
                    const canUseForLinks =
                      browserState.canUseProfileForLinks(profile);
                    const tooltipContent = getProfileTooltipContent(profile);

                    return (
                      <Tooltip key={profile.name}>
                        <TooltipTrigger asChild>
                          <div>
                            <SelectItem
                              value={profile.name}
                              disabled={!canUseForLinks}
                            >
                              <div
                                className={`flex items-center gap-2 ${
                                  !canUseForLinks ? "opacity-50" : ""
                                }`}
                              >
                                <div className="flex gap-3 items-center px-2 py-1 rounded-lg cursor-pointer hover:bg-accent">
                                  <div className="flex gap-2 items-center">
                                    {(() => {
                                      const IconComponent = getBrowserIcon(
                                        profile.browser,
                                      );
                                      return IconComponent ? (
                                        <IconComponent className="w-4 h-4" />
                                      ) : null;
                                    })()}
                                  </div>
                                  <div className="flex-1 text-right">
                                    <div className="font-medium">
                                      {profile.name}
                                    </div>
                                  </div>
                                </div>
                                <Badge variant="secondary" className="text-xs">
                                  {getBrowserDisplayName(profile.browser)}
                                </Badge>
                                {hasProxy(profile) && (
                                  <Badge variant="outline" className="text-xs">
                                    Proxy
                                  </Badge>
                                )}
                                {isRunning && (
                                  <Badge variant="default" className="text-xs">
                                    Running
                                  </Badge>
                                )}
                                {!canUseForLinks && (
                                  <Badge
                                    variant="destructive"
                                    className="text-xs"
                                  >
                                    Unavailable
                                  </Badge>
                                )}
                              </div>
                            </SelectItem>
                          </div>
                        </TooltipTrigger>
                        {tooltipContent && (
                          <TooltipContent>{tooltipContent}</TooltipContent>
                        )}
                      </Tooltip>
                    );
                  })}
                </SelectContent>
              </Select>
            )}
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={handleCancel}>
            Cancel
          </Button>
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="inline-flex">
                <LoadingButton
                  isLoading={isLaunching}
                  onClick={() => void handleOpenUrl()}
                  disabled={
                    !selectedProfile ||
                    profiles.length === 0 ||
                    !canOpenWithSelectedProfile()
                  }
                >
                  Open
                </LoadingButton>
              </span>
            </TooltipTrigger>
            {getTooltipContent() && (
              <TooltipContent>{getTooltipContent()}</TooltipContent>
            )}
          </Tooltip>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
