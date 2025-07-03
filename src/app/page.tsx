"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrent } from "@tauri-apps/plugin-deep-link";
import { useCallback, useEffect, useRef, useState } from "react";
import { FaDownload } from "react-icons/fa";
import { FiWifi } from "react-icons/fi";
import { GoGear, GoKebabHorizontal, GoPlus } from "react-icons/go";
import { ChangeVersionDialog } from "@/components/change-version-dialog";
import { CreateProfileDialog } from "@/components/create-profile-dialog";
import { ImportProfileDialog } from "@/components/import-profile-dialog";
import { PermissionDialog } from "@/components/permission-dialog";
import { ProfilesDataTable } from "@/components/profile-data-table";
import { ProfileSelectorDialog } from "@/components/profile-selector-dialog";
import { ProxyManagementDialog } from "@/components/proxy-management-dialog";
import { ProxySettingsDialog } from "@/components/proxy-settings-dialog";
import { SettingsDialog } from "@/components/settings-dialog";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useAppUpdateNotifications } from "@/hooks/use-app-update-notifications";
import type { PermissionType } from "@/hooks/use-permissions";
import { usePermissions } from "@/hooks/use-permissions";
import { useUpdateNotifications } from "@/hooks/use-update-notifications";
import { useVersionUpdater } from "@/hooks/use-version-updater";
import { showErrorToast } from "@/lib/toast-utils";
import { sleep } from "@/lib/utils";
import type { BrowserProfile } from "@/types";

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
  const [importProfileDialogOpen, setImportProfileDialogOpen] = useState(false);
  const [proxyManagementDialogOpen, setProxyManagementDialogOpen] =
    useState(false);
  const [pendingUrls, setPendingUrls] = useState<PendingUrl[]>([]);
  const [currentProfileForProxy, setCurrentProfileForProxy] =
    useState<BrowserProfile | null>(null);
  const [currentProfileForVersionChange, setCurrentProfileForVersionChange] =
    useState<BrowserProfile | null>(null);
  const [hasCheckedStartupPrompt, setHasCheckedStartupPrompt] = useState(false);
  const [permissionDialogOpen, setPermissionDialogOpen] = useState(false);
  const [currentPermissionType, setCurrentPermissionType] =
    useState<PermissionType>("microphone");
  const [proxyDataReloadTrigger, setProxyDataReloadTrigger] = useState(0);
  const { isMicrophoneAccessGranted, isCameraAccessGranted, isInitialized } =
    usePermissions();

  // Check for missing binaries and offer to download them
  const checkMissingBinaries = useCallback(async () => {
    try {
      const missingBinaries = await invoke<[string, string, string][]>(
        "check_missing_binaries",
      );

      if (missingBinaries.length > 0) {
        console.log("Found missing binaries:", missingBinaries);

        // Group missing binaries by browser type to avoid concurrent downloads
        const browserMap = new Map<string, string[]>();
        for (const [profileName, browser, version] of missingBinaries) {
          if (!browserMap.has(browser)) {
            browserMap.set(browser, []);
          }
          const versions = browserMap.get(browser);
          if (versions) {
            versions.push(`${version} (for ${profileName})`);
          }
        }

        // Show a toast notification about missing binaries and auto-download them
        const missingList = Array.from(browserMap.entries())
          .map(([browser, versions]) => `${browser}: ${versions.join(", ")}`)
          .join(", ");

        console.log(`Downloading missing binaries: ${missingList}`);

        try {
          // Download missing binaries sequentially by browser type to prevent conflicts
          const downloaded = await invoke<string[]>(
            "ensure_all_binaries_exist",
          );
          if (downloaded.length > 0) {
            console.log(
              "Successfully downloaded missing binaries:",
              downloaded,
            );
          }
        } catch (downloadError) {
          console.error("Failed to download missing binaries:", downloadError);
          setError(
            `Failed to download missing binaries: ${JSON.stringify(
              downloadError,
            )}`,
          );
        }
      }
    } catch (err: unknown) {
      console.error("Failed to check missing binaries:", err);
    }
  }, []);

  // Simple profiles loader without updates check (for use as callback)
  const loadProfiles = useCallback(async () => {
    try {
      const profileList = await invoke<BrowserProfile[]>(
        "list_browser_profiles",
      );
      setProfiles(profileList);

      // Check for missing binaries after loading profiles
      await checkMissingBinaries();
    } catch (err: unknown) {
      console.error("Failed to load profiles:", err);
      setError(`Failed to load profiles: ${JSON.stringify(err)}`);
    }
  }, [checkMissingBinaries]);

  // Trigger proxy data reload in ProfilesDataTable
  const triggerProxyDataReload = useCallback(() => {
    setProxyDataReloadTrigger((prev) => prev + 1);
  }, []);

  const handleUrlOpen = useCallback(async (url: string) => {
    try {
      // Use smart profile selection
      const result = await invoke<string>("smart_open_url", {
        url,
      });
      console.log("Smart URL opening succeeded:", result);
      // URL was handled successfully, no need to show selector
    } catch (error: unknown) {
      console.log(
        "Smart URL opening failed or requires profile selection:",
        error,
      );

      // Show profile selector for manual selection
      // Replace any existing pending URL with the new one
      setPendingUrls([{ id: Date.now().toString(), url }]);
    }
  }, []);

  // Version updater for handling version fetching progress events and auto-updates
  useVersionUpdater();

  // Auto-update functionality - use the existing hook for compatibility
  const updateNotifications = useUpdateNotifications(loadProfiles);
  const { checkForUpdates, isUpdating } = updateNotifications;

  // Profiles loader with update check (for initial load and manual refresh)
  const loadProfilesWithUpdateCheck = useCallback(async () => {
    try {
      const profileList = await invoke<BrowserProfile[]>(
        "list_browser_profiles",
      );
      setProfiles(profileList);

      // TODO: remove after a few version bumps, needed to properly display migrated profiles
      setTimeout(async () => {
        for (let i = 0; i < 10; i++) {
          const profiles = await invoke<BrowserProfile[]>(
            "list_browser_profiles",
          );
          setProfiles(profiles);
        }
        await sleep(500);
      }, 0);

      // Check for updates after loading profiles
      await checkForUpdates();
      await checkMissingBinaries();
    } catch (err: unknown) {
      console.error("Failed to load profiles:", err);
      setError(`Failed to load profiles: ${JSON.stringify(err)}`);
    }
  }, [checkForUpdates, checkMissingBinaries]);

  useAppUpdateNotifications();

  // For some reason, app.deep_link().get_current() is not working properly
  const checkCurrentUrl = useCallback(async () => {
    try {
      const currentUrl = await getCurrent();
      if (currentUrl && currentUrl.length > 0) {
        void handleUrlOpen(currentUrl[0]);
      }
    } catch (error) {
      console.error("Failed to check current URL:", error);
    }
  }, [handleUrlOpen]);

  const checkStartupPrompt = useCallback(async () => {
    // Only check once during app startup to prevent reopening after dismissing notifications
    if (hasCheckedStartupPrompt) return;

    try {
      const shouldShow = await invoke<boolean>(
        "should_show_settings_on_startup",
      );
      if (shouldShow) {
        setSettingsDialogOpen(true);
      }
      setHasCheckedStartupPrompt(true);
    } catch (error) {
      console.error("Failed to check startup prompt:", error);
      setHasCheckedStartupPrompt(true);
    }
  }, [hasCheckedStartupPrompt]);

  const checkAllPermissions = useCallback(async () => {
    try {
      // Wait for permissions to be initialized before checking
      if (!isInitialized) {
        return;
      }

      // Check if any permissions are not granted - prioritize missing permissions
      if (!isMicrophoneAccessGranted) {
        setCurrentPermissionType("microphone");
        setPermissionDialogOpen(true);
      } else if (!isCameraAccessGranted) {
        setCurrentPermissionType("camera");
        setPermissionDialogOpen(true);
      }
    } catch (error) {
      console.error("Failed to check permissions:", error);
    }
  }, [isMicrophoneAccessGranted, isCameraAccessGranted, isInitialized]);

  const checkNextPermission = useCallback(() => {
    try {
      if (!isMicrophoneAccessGranted) {
        setCurrentPermissionType("microphone");
        setPermissionDialogOpen(true);
      } else if (!isCameraAccessGranted) {
        setCurrentPermissionType("camera");
        setPermissionDialogOpen(true);
      } else {
        setPermissionDialogOpen(false);
      }
    } catch (error) {
      console.error("Failed to check next permission:", error);
    }
  }, [isMicrophoneAccessGranted, isCameraAccessGranted]);

  const checkStartupUrls = useCallback(async () => {
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
  }, []);

  const listenForUrlEvents = useCallback(async () => {
    try {
      // Listen for URL open events from the deep link handler (when app is already running)
      await listen<string>("url-open-request", (event) => {
        console.log("Received URL open request:", event.payload);
        void handleUrlOpen(event.payload);
      });

      // Listen for show profile selector events
      await listen<string>("show-profile-selector", (event) => {
        console.log("Received show profile selector request:", event.payload);
        setPendingUrls([{ id: Date.now().toString(), url: event.payload }]);
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
  }, [handleUrlOpen]);

  const openProxyDialog = useCallback((profile: BrowserProfile | null) => {
    setCurrentProfileForProxy(profile);
    setProxyDialogOpen(true);
  }, []);

  const openChangeVersionDialog = useCallback((profile: BrowserProfile) => {
    setCurrentProfileForVersionChange(profile);
    setChangeVersionDialogOpen(true);
  }, []);

  const handleSaveProxy = useCallback(
    async (proxyId: string | null) => {
      setProxyDialogOpen(false);
      setError(null);

      try {
        if (currentProfileForProxy) {
          await invoke("update_profile_proxy", {
            profileName: currentProfileForProxy.name,
            proxyId: proxyId,
          });
        }
        await loadProfiles();
        // Trigger proxy data reload in the table
        triggerProxyDataReload();
      } catch (err: unknown) {
        console.error("Failed to update proxy settings:", err);
        setError(`Failed to update proxy settings: ${JSON.stringify(err)}`);
      }
    },
    [currentProfileForProxy, loadProfiles, triggerProxyDataReload],
  );

  const handleCreateProfile = useCallback(
    async (profileData: {
      name: string;
      browserStr: BrowserTypeString;
      version: string;
      releaseType: string;
      proxyId?: string;
    }) => {
      setError(null);

      try {
        const _profile = await invoke<BrowserProfile>(
          "create_browser_profile_new",
          {
            name: profileData.name,
            browserStr: profileData.browserStr,
            version: profileData.version,
            releaseType: profileData.releaseType,
            proxyId: profileData.proxyId,
          },
        );

        await loadProfiles();
        // Trigger proxy data reload in the table
        triggerProxyDataReload();
      } catch (error) {
        setError(
          `Failed to create profile: ${
            error instanceof Error ? error.message : String(error)
          }`,
        );
        throw error;
      }
    },
    [loadProfiles, triggerProxyDataReload],
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

  const handleDeleteProfile = useCallback(
    async (profile: BrowserProfile) => {
      setError(null);
      console.log("Attempting to delete profile:", profile.name);

      try {
        // First check if the browser is running for this profile
        const isRunning = await invoke<boolean>("check_browser_status", {
          profile,
        });

        if (isRunning) {
          setError(
            "Cannot delete profile while browser is running. Please stop the browser first.",
          );
          return;
        }

        // Attempt to delete the profile
        await invoke("delete_profile", { profileName: profile.name });
        console.log("Profile deletion command completed successfully");

        // Give a small delay to ensure file system operations complete
        await new Promise((resolve) => setTimeout(resolve, 100));

        // Reload profiles to ensure UI is updated
        await loadProfiles();

        console.log("Profile deleted and profiles reloaded successfully");
      } catch (err: unknown) {
        console.error("Failed to delete profile:", err);
        const errorMessage = err instanceof Error ? err.message : String(err);
        setError(`Failed to delete profile: ${errorMessage}`);
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

  useEffect(() => {
    void loadProfilesWithUpdateCheck();

    // Check for startup default browser prompt
    void checkStartupPrompt();

    // Listen for URL open events
    void listenForUrlEvents();

    // Check for startup URLs (when app was launched as default browser)
    void checkStartupUrls();
    void checkCurrentUrl();

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
  }, [
    loadProfilesWithUpdateCheck,
    checkForUpdates,
    checkCurrentUrl,
    checkStartupPrompt,
    listenForUrlEvents,
    checkStartupUrls,
  ]);

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

  // Check permissions when they are initialized
  useEffect(() => {
    if (isInitialized) {
      void checkAllPermissions();
    }
  }, [isInitialized, checkAllPermissions]);

  return (
    <div className="grid grid-rows-[20px_1fr_20px] items-center justify-items-center min-h-screen gap-8 font-[family-name:var(--font-geist-sans)]  bg-white dark:bg-black">
      <main className="flex flex-col row-start-2 gap-8 items-center w-full max-w-3xl">
        <Card className="w-full">
          <CardHeader>
            <div className="flex justify-between items-center">
              <CardTitle>Profiles</CardTitle>
              <div className="flex gap-2 items-center">
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      size="sm"
                      variant="outline"
                      className="flex gap-2 items-center"
                    >
                      <GoKebabHorizontal className="w-4 h-4" />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem
                      onClick={() => {
                        setSettingsDialogOpen(true);
                      }}
                    >
                      <GoGear className="mr-2 w-4 h-4" />
                      Settings
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      onClick={() => {
                        setProxyManagementDialogOpen(true);
                      }}
                    >
                      <FiWifi className="mr-2 w-4 h-4" />
                      Proxies
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      onClick={() => {
                        setImportProfileDialogOpen(true);
                      }}
                    >
                      <FaDownload className="mr-2 w-4 h-4" />
                      Import Profile
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      size="sm"
                      onClick={() => {
                        setCreateProfileDialogOpen(true);
                      }}
                      className="flex gap-2 items-center"
                    >
                      <GoPlus className="w-4 h-4" />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Create a new profile</TooltipContent>
                </Tooltip>
              </div>
            </div>
          </CardHeader>
          <CardContent>
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
              onReloadProxyData={
                proxyDataReloadTrigger > 0 ? triggerProxyDataReload : undefined
              }
            />
          </CardContent>
        </Card>
      </main>

      <ProxySettingsDialog
        isOpen={proxyDialogOpen}
        onClose={() => {
          setProxyDialogOpen(false);
        }}
        onSave={handleSaveProxy}
        initialProxyId={currentProfileForProxy?.proxy_id}
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

      <ImportProfileDialog
        isOpen={importProfileDialogOpen}
        onClose={() => {
          setImportProfileDialogOpen(false);
        }}
        onImportComplete={() => void loadProfiles()}
      />

      <ProxyManagementDialog
        isOpen={proxyManagementDialogOpen}
        onClose={() => {
          setProxyManagementDialogOpen(false);
        }}
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

      <PermissionDialog
        isOpen={permissionDialogOpen}
        onClose={() => {
          setPermissionDialogOpen(false);
        }}
        permissionType={currentPermissionType}
        onPermissionGranted={checkNextPermission}
      />
    </div>
  );
}
