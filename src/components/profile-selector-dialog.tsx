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
import { getBrowserDisplayName, getBrowserIcon } from "@/lib/browser-utils";
import type { BrowserProfile } from "@/types";

interface ProfileSelectorDialogProps {
  isOpen: boolean;
  onClose: () => void;
  url?: string;
  runningProfiles?: Set<string>;
}

export function ProfileSelectorDialog({
  isOpen,
  onClose,
  url,
  runningProfiles = new Set(),
}: ProfileSelectorDialogProps) {
  const [profiles, setProfiles] = useState<BrowserProfile[]>([]);
  const [selectedProfile, setSelectedProfile] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isLaunching, setIsLaunching] = useState(false);

  // Helper function to determine if a profile can be used for opening links
  const canUseProfileForLinks = useCallback(
    (
      profile: BrowserProfile,
      allProfiles: BrowserProfile[],
      runningProfiles: Set<string>,
    ): boolean => {
      const isRunning = runningProfiles.has(profile.name);

      // For TOR browser: Check if any TOR browser is running
      if (profile.browser === "tor-browser") {
        const runningTorProfiles = allProfiles.filter(
          (p) => p.browser === "tor-browser" && runningProfiles.has(p.name),
        );

        // If no TOR browser is running, allow any TOR profile
        if (runningTorProfiles.length === 0) {
          return true;
        }

        // If TOR browser(s) are running, only allow the running one(s)
        return isRunning;
      }

      // For Mullvad browser: never allow if running
      if (profile.browser === "mullvad-browser" && isRunning) {
        return false;
      }

      // For other browsers: always allow
      return true;
    },
    [],
  );

  const loadProfiles = useCallback(async () => {
    setIsLoading(true);
    try {
      const profileList = await invoke<BrowserProfile[]>(
        "list_browser_profiles",
      );

      // Sort profiles by name
      profileList.sort((a, b) => a.name.localeCompare(b.name));

      // Don't filter any profiles, show all of them
      setProfiles(profileList);

      // Auto-select first available profile for link opening
      if (profileList.length > 0) {
        // First, try to find a running profile that can be used for opening links
        const runningAvailableProfile = profileList.find((profile) => {
          const isRunning = runningProfiles.has(profile.name);
          return (
            isRunning &&
            canUseProfileForLinks(profile, profileList, runningProfiles)
          );
        });

        if (runningAvailableProfile) {
          setSelectedProfile(runningAvailableProfile.name);
        } else {
          // If no running profile is suitable, find the first profile that can be used for opening links
          const availableProfile = profileList.find((profile) => {
            return canUseProfileForLinks(profile, profileList, runningProfiles);
          });

          if (availableProfile) {
            setSelectedProfile(availableProfile.name);
          } else {
            // If no suitable profile found, still select the first one to show UI
            setSelectedProfile(profileList[0].name);
          }
        }
      }
    } catch (error) {
      console.error("Failed to load profiles:", error);
    } finally {
      setIsLoading(false);
    }
  }, [runningProfiles, canUseProfileForLinks]);

  // Helper function to get tooltip content for profiles
  const getProfileTooltipContent = (profile: BrowserProfile): string => {
    const isRunning = runningProfiles.has(profile.name);

    if (profile.browser === "tor-browser") {
      // If another TOR profile is running, this one is not available
      return "Only 1 instance can run at a time";
    }

    if (profile.browser === "mullvad-browser") {
      if (isRunning) {
        return "Only launching the browser is supported, opening them in a running browser is not yet available";
      }
      return "Only launching the browser is supported, opening them in a running browser is not yet available";
    }

    if (isRunning) {
      return "URL will open in a new tab in the existing browser window";
    }

    return "";
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
    return canUseProfileForLinks(
      selectedProfileData,
      profiles,
      runningProfiles,
    );
  };

  // Get tooltip content for disabled profiles
  const getTooltipContent = () => {
    if (!selectedProfileData) return "";
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
                    const canUseForLinks = canUseProfileForLinks(
                      profile,
                      profiles,
                      runningProfiles,
                    );
                    const tooltipContent = getProfileTooltipContent(profile);

                    return (
                      <Tooltip key={profile.name}>
                        <TooltipTrigger asChild>
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
                              {profile.proxy?.enabled && (
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
              <div>
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
              </div>
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
