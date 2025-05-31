"use client";

import { ChangeVersionDialog } from "@/components/change-version-dialog";
import { CreateProfileDialog } from "@/components/create-profile-dialog";
import { ProfilesDataTable } from "@/components/profile-data-table";
import { ProfileSelectorDialog } from "@/components/profile-selector-dialog";
import { ProxySettingsDialog } from "@/components/proxy-settings-dialog";
import { SettingsDialog } from "@/components/settings-dialog";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useAppUpdateNotifications } from "@/hooks/use-app-update-notifications";
import { useUpdateNotifications } from "@/hooks/use-update-notifications";
import { showErrorToast } from "@/lib/toast-utils";
import type { BrowserProfile, ProxySettings } from "@/types";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { GoGear, GoPlus } from "react-icons/go";

type BrowserTypeString =
  | "mullvad-browser"
  | "firefox"
  | "firefox-developer"
  | "chromium"
  | "brave"
  | "zen"
  | "tor-browser";

interface PendingUrl {
  id: string;
  url: string;
}

export default function Home() {
  const [profiles, setProfiles] = useState<BrowserProfile[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [proxyDialogOpen, setProxyDialogOpen] = useState(false);
  const [createProfileDialogOpen, setCreateProfileDialogOpen] = useState(false);
  const [changeVersionDialogOpen, setChangeVersionDialogOpen] = useState(false);
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false);
  const [pendingUrls, setPendingUrls] = useState<PendingUrl[]>([]);
  const [currentProfileForProxy, setCurrentProfileForProxy] =
    useState<BrowserProfile | null>(null);
  const [currentProfileForVersionChange, setCurrentProfileForVersionChange] =
    useState<BrowserProfile | null>(null);

  // Simple profiles loader without updates check (for use as callback)
  const loadProfiles = useCallback(async () => {
    try {
      const profileList = await invoke<BrowserProfile[]>(
        "list_browser_profiles",
      );
      setProfiles(profileList);
    } catch (err: unknown) {
      console.error("Failed to load profiles:", err);
      setError(`Failed to load profiles: ${JSON.stringify(err)}`);
    }
  }, []);

  // Auto-update functionality - pass loadProfiles to refresh profiles after updates
  const updateNotifications = useUpdateNotifications(loadProfiles);
  const { checkForUpdates, isUpdating } = updateNotifications;

  // Profiles loader with update check (for initial load and manual refresh)
  const loadProfilesWithUpdateCheck = useCallback(async () => {
    try {
      const profileList = await invoke<BrowserProfile[]>(
        "list_browser_profiles",
      );
      setProfiles(profileList);

      // Check for updates after loading profiles
      await checkForUpdates();
    } catch (err: unknown) {
      console.error("Failed to load profiles:", err);
      setError(`Failed to load profiles: ${JSON.stringify(err)}`);
    }
  }, [checkForUpdates]);

  useAppUpdateNotifications();

  useEffect(() => {
    void loadProfilesWithUpdateCheck();

    // Check for startup default browser prompt
    void checkStartupPrompt();

    // Listen for URL open events
    void listenForUrlEvents();

    // Check for startup URLs (when app was launched as default browser)
    void checkStartupUrls();

    // Set up periodic update checks (every 30 minutes)
    const updateInterval = setInterval(
      () => {
        void checkForUpdates();
      },
      30 * 60 * 1000,
    );

    return () => {
      clearInterval(updateInterval);
    };
  }, [loadProfilesWithUpdateCheck, checkForUpdates]);

  const checkStartupPrompt = async () => {
    try {
      const shouldShow = await invoke<boolean>(
        "should_show_settings_on_startup",
      );
      if (shouldShow) {
        setSettingsDialogOpen(true);
      }
    } catch (error) {
      console.error("Failed to check startup prompt:", error);
    }
  };

  const checkStartupUrls = async () => {
    try {
      const hasStartupUrl = await invoke<boolean>(
        "check_and_handle_startup_url",
      );
      if (hasStartupUrl) {
        console.log("Handled startup URL successfully");
      }
    } catch (error) {
      console.error("Failed to check startup URLs:", error);
    }
  };

  const listenForUrlEvents = async () => {
    try {
      // Listen for URL open events from the deep link handler (when app is already running)
      await listen<string>("url-open-request", (event) => {
        console.log("Received URL open request:", event.payload);
        void handleUrlOpen(event.payload);
      });

      // Listen for show profile selector events
      await listen<string>("show-profile-selector", (event) => {
        console.log("Received show profile selector request:", event.payload);
        setPendingUrls((prev) => [
          ...prev,
          { id: Date.now().toString(), url: event.payload },
        ]);
      });

      // Listen for show create profile dialog events
      await listen<string>("show-create-profile-dialog", (event) => {
        console.log(
          "Received show create profile dialog request:",
          event.payload,
        );
        setError(
          "No profiles available. Please create a profile first before opening URLs.",
        );
        setCreateProfileDialogOpen(true);
      });
    } catch (error) {
      console.error("Failed to setup URL listener:", error);
    }
  };

  const handleUrlOpen = async (url: string) => {
    try {
      // Use smart profile selection
      const result = await invoke<string>("smart_open_url", {
        url,
      });
      console.log("Smart URL opening succeeded:", result);
      // URL was handled successfully
    } catch (error: unknown) {
      console.log(
        "Smart URL opening failed or requires profile selection:",
        error,
      );

      // Show profile selector for manual selection
      setPendingUrls((prev) => [...prev, { id: Date.now().toString(), url }]);
    }
  };

  const openProxyDialog = useCallback((profile: BrowserProfile | null) => {
    setCurrentProfileForProxy(profile);
    setProxyDialogOpen(true);
  }, []);

  const openChangeVersionDialog = useCallback((profile: BrowserProfile) => {
    setCurrentProfileForVersionChange(profile);
    setChangeVersionDialogOpen(true);
  }, []);

  const handleSaveProxy = useCallback(
    async (proxySettings: ProxySettings) => {
      setProxyDialogOpen(false);
      setError(null);

      try {
        if (currentProfileForProxy) {
          await invoke("update_profile_proxy", {
            profileName: currentProfileForProxy.name,
            proxy: proxySettings,
          });
        }
        await loadProfiles();
      } catch (err: unknown) {
        console.error("Failed to update proxy settings:", err);
        setError(`Failed to update proxy settings: ${JSON.stringify(err)}`);
      }
    },
    [currentProfileForProxy, loadProfiles],
  );

  const handleCreateProfile = useCallback(
    async (profileData: {
      name: string;
      browserStr: BrowserTypeString;
      version: string;
      proxy?: ProxySettings;
    }) => {
      setError(null);

      try {
        const profile = await invoke<BrowserProfile>(
          "create_browser_profile_new",
          {
            name: profileData.name,
            browserStr: profileData.browserStr,
            version: profileData.version,
          },
        );

        // Update proxy if provided
        if (profileData.proxy) {
          await invoke("update_profile_proxy", {
            profileName: profile.name,
            proxy: profileData.proxy,
          });
        }

        await loadProfiles();
      } catch (error) {
        setError(
          `Failed to create profile: ${
            error instanceof Error ? error.message : String(error)
          }`,
        );
        throw error;
      }
    },
    [loadProfiles],
  );

  const [runningProfiles, setRunningProfiles] = useState<Set<string>>(
    new Set(),
  );

  const runningProfilesRef = useRef<Set<string>>(new Set());

  const checkBrowserStatus = useCallback(async (profile: BrowserProfile) => {
    try {
      const isRunning = await invoke<boolean>("check_browser_status", {
        profile,
      });

      const currentRunning = runningProfilesRef.current.has(profile.name);

      if (isRunning !== currentRunning) {
        setRunningProfiles((prev) => {
          const next = new Set(prev);
          if (isRunning) {
            next.add(profile.name);
          } else {
            next.delete(profile.name);
          }
          runningProfilesRef.current = next;
          return next;
        });
      }
    } catch (err) {
      console.error("Failed to check browser status:", err);
    }
  }, []);

  const launchProfile = useCallback(
    async (profile: BrowserProfile) => {
      setError(null);

      // Check if browser is disabled due to ongoing update
      try {
        const isDisabled = await invoke<boolean>(
          "is_browser_disabled_for_update",
          {
            browser: profile.browser,
          },
        );

        if (isDisabled || isUpdating(profile.browser)) {
          setError(
            `${profile.browser} is currently being updated. Please wait for the update to complete.`,
          );
          return;
        }
      } catch (err) {
        console.error("Failed to check browser update status:", err);
      }

      try {
        const updatedProfile = await invoke<BrowserProfile>(
          "launch_browser_profile",
          { profile },
        );
        await loadProfiles();
        await checkBrowserStatus(updatedProfile);
      } catch (err: unknown) {
        console.error("Failed to launch browser:", err);
        setError(`Failed to launch browser: ${JSON.stringify(err)}`);
      }
    },
    [loadProfiles, checkBrowserStatus, isUpdating],
  );

  useEffect(() => {
    if (profiles.length === 0) return;

    const interval = setInterval(() => {
      for (const profile of profiles) {
        void checkBrowserStatus(profile);
      }
    }, 500);

    return () => {
      clearInterval(interval);
    };
  }, [profiles, checkBrowserStatus]);

  useEffect(() => {
    runningProfilesRef.current = runningProfiles;
  }, [runningProfiles]);

  useEffect(() => {
    if (error) {
      showErrorToast(error);
      setError(null);
    }
  }, [error]);

  const handleDeleteProfile = useCallback(
    async (profile: BrowserProfile) => {
      setError(null);
      try {
        await invoke("delete_profile", { profileName: profile.name });
        await loadProfiles();
      } catch (err: unknown) {
        console.error("Failed to delete profile:", err);
        setError(`Failed to delete profile: ${JSON.stringify(err)}`);
      }
    },
    [loadProfiles],
  );

  const handleRenameProfile = useCallback(
    async (oldName: string, newName: string) => {
      setError(null);
      try {
        await invoke("rename_profile", { oldName, newName });
        await loadProfiles();
      } catch (err: unknown) {
        console.error("Failed to rename profile:", err);
        setError(`Failed to rename profile: ${JSON.stringify(err)}`);
        throw err;
      }
    },
    [loadProfiles],
  );

  const handleKillProfile = useCallback(
    async (profile: BrowserProfile) => {
      setError(null);
      try {
        await invoke("kill_browser_profile", { profile });
        await loadProfiles();
      } catch (err: unknown) {
        console.error("Failed to kill browser:", err);
        setError(`Failed to kill browser: ${JSON.stringify(err)}`);
      }
    },
    [loadProfiles],
  );

  return (
    <div className="grid grid-rows-[20px_1fr_20px] items-center justify-items-center min-h-screen p-8 gap-8 sm:p-12 font-[family-name:var(--font-geist-sans)]">
      <main className="flex flex-col gap-8 row-start-2 items-center w-full max-w-3xl">
        <Card className="w-full">
          <CardHeader>
            <div className="flex items-center justify-between">
              <CardTitle>Profiles</CardTitle>
              <div className="flex items-center gap-2">
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => {
                        setSettingsDialogOpen(true);
                      }}
                      className="flex items-center gap-2"
                    >
                      <GoGear className="h-4 w-4" />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Settings</TooltipContent>
                </Tooltip>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      size="sm"
                      onClick={() => {
                        setCreateProfileDialogOpen(true);
                      }}
                      className="flex items-center gap-2"
                    >
                      <GoPlus className="h-4 w-4" />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Create a new profile</TooltipContent>
                </Tooltip>
              </div>
            </div>
          </CardHeader>
          <CardContent className="space-y-6">
            <ProfilesDataTable
              data={profiles}
              onLaunchProfile={launchProfile}
              onKillProfile={handleKillProfile}
              onProxySettings={openProxyDialog}
              onDeleteProfile={handleDeleteProfile}
              onRenameProfile={handleRenameProfile}
              onChangeVersion={openChangeVersionDialog}
              runningProfiles={runningProfiles}
              isUpdating={isUpdating}
            />
          </CardContent>
        </Card>
      </main>

      <ProxySettingsDialog
        isOpen={proxyDialogOpen}
        onClose={() => {
          setProxyDialogOpen(false);
        }}
        onSave={(proxy: ProxySettings) => void handleSaveProxy(proxy)}
        initialSettings={currentProfileForProxy?.proxy}
        browserType={currentProfileForProxy?.browser}
      />

      <CreateProfileDialog
        isOpen={createProfileDialogOpen}
        onClose={() => {
          setCreateProfileDialogOpen(false);
        }}
        onCreateProfile={handleCreateProfile}
      />

      <SettingsDialog
        isOpen={settingsDialogOpen}
        onClose={() => {
          setSettingsDialogOpen(false);
        }}
      />

      <ChangeVersionDialog
        isOpen={changeVersionDialogOpen}
        onClose={() => {
          setChangeVersionDialogOpen(false);
        }}
        profile={currentProfileForVersionChange}
        onVersionChanged={() => void loadProfiles()}
      />

      {pendingUrls.map((pendingUrl) => (
        <ProfileSelectorDialog
          key={pendingUrl.id}
          isOpen={true}
          onClose={() => {
            setPendingUrls((prev) =>
              prev.filter((u) => u.id !== pendingUrl.id),
            );
          }}
          url={pendingUrl.url}
          runningProfiles={runningProfiles}
        />
      ))}
    </div>
  );
}
