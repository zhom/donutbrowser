"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrent } from "@tauri-apps/plugin-deep-link";
import { useCallback, useEffect, useRef, useState } from "react";
import { CamoufoxConfigDialog } from "@/components/camoufox-config-dialog";
import { ChangeVersionDialog } from "@/components/change-version-dialog";
import { CreateProfileDialog } from "@/components/create-profile-dialog";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { GroupAssignmentDialog } from "@/components/group-assignment-dialog";
import { GroupBadges } from "@/components/group-badges";
import { GroupManagementDialog } from "@/components/group-management-dialog";
import HomeHeader from "@/components/home-header";
import { ImportProfileDialog } from "@/components/import-profile-dialog";
import { PermissionDialog } from "@/components/permission-dialog";
import { ProfilesDataTable } from "@/components/profile-data-table";
import { ProfileSelectorDialog } from "@/components/profile-selector-dialog";
import { ProxyManagementDialog } from "@/components/proxy-management-dialog";
import { ProxySettingsDialog } from "@/components/proxy-settings-dialog";
import { SettingsDialog } from "@/components/settings-dialog";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { useAppUpdateNotifications } from "@/hooks/use-app-update-notifications";
import type { PermissionType } from "@/hooks/use-permissions";
import { usePermissions } from "@/hooks/use-permissions";
import { useUpdateNotifications } from "@/hooks/use-update-notifications";
import { showErrorToast } from "@/lib/toast-utils";
import type { BrowserProfile, CamoufoxConfig, GroupWithCount } from "@/types";

type BrowserTypeString =
  | "mullvad-browser"
  | "firefox"
  | "firefox-developer"
  | "chromium"
  | "brave"
  | "zen"
  | "tor-browser"
  | "camoufox";

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
  const [camoufoxConfigDialogOpen, setCamoufoxConfigDialogOpen] =
    useState(false);
  const [groupManagementDialogOpen, setGroupManagementDialogOpen] =
    useState(false);
  const [groupAssignmentDialogOpen, setGroupAssignmentDialogOpen] =
    useState(false);
  const [selectedGroupId, setSelectedGroupId] = useState<string>("default");
  const [selectedProfilesForGroup, setSelectedProfilesForGroup] = useState<
    string[]
  >([]);
  const [selectedProfiles, setSelectedProfiles] = useState<string[]>([]);
  const [pendingUrls, setPendingUrls] = useState<PendingUrl[]>([]);
  const [currentProfileForProxy, setCurrentProfileForProxy] =
    useState<BrowserProfile | null>(null);
  const [currentProfileForVersionChange, setCurrentProfileForVersionChange] =
    useState<BrowserProfile | null>(null);
  const [currentProfileForCamoufoxConfig, setCurrentProfileForCamoufoxConfig] =
    useState<BrowserProfile | null>(null);
  const [hasCheckedStartupPrompt, setHasCheckedStartupPrompt] = useState(false);
  const [permissionDialogOpen, setPermissionDialogOpen] = useState(false);
  const [groups, setGroups] = useState<GroupWithCount[]>([]);
  const [areGroupsLoading, setGroupsLoading] = useState(true);
  const [currentPermissionType, setCurrentPermissionType] =
    useState<PermissionType>("microphone");
  const [showBulkDeleteConfirmation, setShowBulkDeleteConfirmation] =
    useState(false);
  const [isBulkDeleting, setIsBulkDeleting] = useState(false);
  const { isMicrophoneAccessGranted, isCameraAccessGranted, isInitialized } =
    usePermissions();

  const handleSelectGroup = useCallback((groupId: string) => {
    setSelectedGroupId(groupId);
    setSelectedProfiles([]);
  }, []);

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

  const [processingUrls, setProcessingUrls] = useState<Set<string>>(new Set());

  const handleUrlOpen = useCallback(
    async (url: string) => {
      // Prevent duplicate processing of the same URL
      if (processingUrls.has(url)) {
        console.log("URL already being processed:", url);
        return;
      }

      setProcessingUrls((prev) => new Set(prev).add(url));

      try {
        console.log("URL received for opening:", url);

        // Always show profile selector for manual selection - never auto-open
        // Replace any existing pending URL with the new one
        setPendingUrls([{ id: Date.now().toString(), url }]);
      } finally {
        // Remove URL from processing set after a short delay to prevent rapid duplicates
        setTimeout(() => {
          setProcessingUrls((prev) => {
            const next = new Set(prev);
            next.delete(url);
            return next;
          });
        }, 1000);
      }
    },
    [processingUrls],
  );

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

      // Check for updates after loading profiles
      await checkForUpdates();
      await checkMissingBinaries();
    } catch (err: unknown) {
      console.error("Failed to load profiles:", err);
      setError(`Failed to load profiles: ${JSON.stringify(err)}`);
    }
  }, [checkForUpdates, checkMissingBinaries]);

  useAppUpdateNotifications();

  // Check for startup URLs but only process them once
  const [hasCheckedStartupUrl, setHasCheckedStartupUrl] = useState(false);
  const checkCurrentUrl = useCallback(async () => {
    if (hasCheckedStartupUrl) return;

    try {
      const currentUrl = await getCurrent();
      if (currentUrl && currentUrl.length > 0) {
        console.log("Startup URL detected:", currentUrl[0]);
        void handleUrlOpen(currentUrl[0]);
      }
    } catch (error) {
      console.error("Failed to check current URL:", error);
    } finally {
      setHasCheckedStartupUrl(true);
    }
  }, [handleUrlOpen, hasCheckedStartupUrl]);

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
    } catch (error) {
      console.error("Failed to check startup prompt:", error);
    } finally {
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
        void handleUrlOpen(event.payload);
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

      // Listen for custom logo click events
      const handleLogoUrlEvent = (event: CustomEvent) => {
        console.log("Received logo URL event:", event.detail);
        void handleUrlOpen(event.detail);
      };

      window.addEventListener(
        "url-open-request",
        handleLogoUrlEvent as EventListener,
      );

      // Return cleanup function
      return () => {
        window.removeEventListener(
          "url-open-request",
          handleLogoUrlEvent as EventListener,
        );
      };
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

  const handleConfigureCamoufox = useCallback((profile: BrowserProfile) => {
    setCurrentProfileForCamoufoxConfig(profile);
    setCamoufoxConfigDialogOpen(true);
  }, []);

  const handleSaveCamoufoxConfig = useCallback(
    async (profile: BrowserProfile, config: CamoufoxConfig) => {
      setError(null);
      try {
        await invoke("update_camoufox_config", {
          profileName: profile.name,
          config,
        });
        await loadProfiles();
        setCamoufoxConfigDialogOpen(false);
      } catch (err: unknown) {
        console.error("Failed to update camoufox config:", err);
        setError(`Failed to update camoufox config: ${JSON.stringify(err)}`);
        throw err;
      }
    },
    [loadProfiles],
  );

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
      } catch (err: unknown) {
        console.error("Failed to update proxy settings:", err);
        setError(`Failed to update proxy settings: ${JSON.stringify(err)}`);
      }
    },
    [currentProfileForProxy, loadProfiles],
  );

  const loadGroups = useCallback(async () => {
    setGroupsLoading(true);
    try {
      const groupsWithCounts = await invoke<GroupWithCount[]>(
        "get_groups_with_profile_counts",
      );
      setGroups(groupsWithCounts);
    } catch (err) {
      console.error("Failed to load groups with counts:", err);
      setGroups([]);
    } finally {
      setGroupsLoading(false);
    }
  }, []);

  const handleCreateProfile = useCallback(
    async (profileData: {
      name: string;
      browserStr: BrowserTypeString;
      version: string;
      releaseType: string;
      proxyId?: string;
      camoufoxConfig?: CamoufoxConfig;
      groupId?: string;
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
            camoufoxConfig: profileData.camoufoxConfig,
            groupId:
              profileData.groupId ||
              (selectedGroupId !== "default" ? selectedGroupId : undefined),
          },
        );

        await loadProfiles();
        await loadGroups();
        // Trigger proxy data reload in the table
      } catch (error) {
        setError(
          `Failed to create profile: ${
            error instanceof Error ? error.message : String(error)
          }`,
        );
        throw error;
      }
    },
    [loadProfiles, loadGroups, selectedGroupId],
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
        console.log(
          `Profile ${profile.name} (${profile.browser}) status changed: ${currentRunning} -> ${isRunning}`,
        );
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
        await new Promise((resolve) => setTimeout(resolve, 500));

        // Reload profiles and groups to ensure UI is updated
        await loadProfiles();
        await loadGroups();

        console.log("Profile deleted and profiles reloaded successfully");
      } catch (err: unknown) {
        console.error("Failed to delete profile:", err);
        const errorMessage = err instanceof Error ? err.message : String(err);
        setError(`Failed to delete profile: ${errorMessage}`);
      }
    },
    [loadProfiles, loadGroups],
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

  const handleDeleteSelectedProfiles = useCallback(
    async (profileNames: string[]) => {
      setError(null);
      try {
        await invoke("delete_selected_profiles", { profileNames });
        await loadProfiles();
        await loadGroups();
      } catch (err: unknown) {
        console.error("Failed to delete selected profiles:", err);
        setError(`Failed to delete selected profiles: ${JSON.stringify(err)}`);
      }
    },
    [loadProfiles, loadGroups],
  );

  const handleAssignProfilesToGroup = useCallback((profileNames: string[]) => {
    setSelectedProfilesForGroup(profileNames);
    setGroupAssignmentDialogOpen(true);
  }, []);

  const handleBulkDelete = useCallback(() => {
    if (selectedProfiles.length === 0) return;
    setShowBulkDeleteConfirmation(true);
  }, [selectedProfiles]);

  const confirmBulkDelete = useCallback(async () => {
    if (selectedProfiles.length === 0) return;

    setIsBulkDeleting(true);
    try {
      await invoke("delete_selected_profiles", {
        profileNames: selectedProfiles,
      });
      await loadProfiles();
      await loadGroups();
      setSelectedProfiles([]);
      setShowBulkDeleteConfirmation(false);
    } catch (error) {
      console.error("Failed to delete selected profiles:", error);
      setError(`Failed to delete selected profiles: ${JSON.stringify(error)}`);
    } finally {
      setIsBulkDeleting(false);
    }
  }, [selectedProfiles, loadProfiles, loadGroups]);

  const handleBulkGroupAssignment = useCallback(() => {
    if (selectedProfiles.length === 0) return;
    handleAssignProfilesToGroup(selectedProfiles);
    setSelectedProfiles([]);
  }, [selectedProfiles, handleAssignProfilesToGroup]);

  const handleGroupAssignmentComplete = useCallback(async () => {
    await loadProfiles();
    await loadGroups();
    setGroupAssignmentDialogOpen(false);
    setSelectedProfilesForGroup([]);
  }, [loadProfiles, loadGroups]);

  const handleGroupManagementComplete = useCallback(async () => {
    await loadGroups();
  }, [loadGroups]);

  useEffect(() => {
    void loadProfilesWithUpdateCheck();
    void loadGroups();

    // Check for startup default browser prompt
    void checkStartupPrompt();

    // Listen for URL open events and get cleanup function
    const setupListeners = async () => {
      const cleanup = await listenForUrlEvents();
      return cleanup;
    };

    let cleanup: (() => void) | undefined;
    setupListeners().then((cleanupFn) => {
      cleanup = cleanupFn;
    });

    // Check for startup URLs (when app was launched as default browser)
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
      if (cleanup) {
        cleanup();
      }
    };
  }, [
    loadProfilesWithUpdateCheck,
    checkForUpdates,
    checkStartupPrompt,
    listenForUrlEvents,
    checkCurrentUrl,
    loadGroups,
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
        <Card className="gap-2 w-full">
          <CardHeader>
            <HomeHeader
              selectedProfiles={selectedProfiles}
              onBulkDelete={handleBulkDelete}
              onBulkGroupAssignment={handleBulkGroupAssignment}
              onCreateProfileDialogOpen={setCreateProfileDialogOpen}
              onGroupManagementDialogOpen={setGroupManagementDialogOpen}
              onImportProfileDialogOpen={setImportProfileDialogOpen}
              onProxyManagementDialogOpen={setProxyManagementDialogOpen}
              onSettingsDialogOpen={setSettingsDialogOpen}
            />
          </CardHeader>
          <CardContent>
            <GroupBadges
              selectedGroupId={selectedGroupId}
              onGroupSelect={handleSelectGroup}
              groups={groups}
              isLoading={areGroupsLoading}
            />
            <ProfilesDataTable
              data={profiles}
              onLaunchProfile={launchProfile}
              onKillProfile={handleKillProfile}
              onProxySettings={openProxyDialog}
              onDeleteProfile={handleDeleteProfile}
              onRenameProfile={handleRenameProfile}
              onChangeVersion={openChangeVersionDialog}
              onConfigureCamoufox={handleConfigureCamoufox}
              runningProfiles={runningProfiles}
              isUpdating={isUpdating}
              onDeleteSelectedProfiles={handleDeleteSelectedProfiles}
              onAssignProfilesToGroup={handleAssignProfilesToGroup}
              selectedGroupId={selectedGroupId}
              selectedProfiles={selectedProfiles}
              onSelectedProfilesChange={setSelectedProfiles}
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
        selectedGroupId={selectedGroupId}
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
          isUpdating={isUpdating}
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

      <CamoufoxConfigDialog
        isOpen={camoufoxConfigDialogOpen}
        onClose={() => {
          setCamoufoxConfigDialogOpen(false);
        }}
        profile={currentProfileForCamoufoxConfig}
        onSave={handleSaveCamoufoxConfig}
      />

      <GroupManagementDialog
        isOpen={groupManagementDialogOpen}
        onClose={() => {
          setGroupManagementDialogOpen(false);
        }}
        onGroupManagementComplete={handleGroupManagementComplete}
      />

      <GroupAssignmentDialog
        isOpen={groupAssignmentDialogOpen}
        onClose={() => {
          setGroupAssignmentDialogOpen(false);
        }}
        selectedProfiles={selectedProfilesForGroup}
        onAssignmentComplete={handleGroupAssignmentComplete}
      />

      <DeleteConfirmationDialog
        isOpen={showBulkDeleteConfirmation}
        onClose={() => setShowBulkDeleteConfirmation(false)}
        onConfirm={confirmBulkDelete}
        title="Delete Selected Profiles"
        description={`This action cannot be undone. This will permanently delete ${selectedProfiles.length} profile${selectedProfiles.length !== 1 ? "s" : ""} and all associated data.`}
        confirmButtonText={`Delete ${selectedProfiles.length} Profile${selectedProfiles.length !== 1 ? "s" : ""}`}
        isLoading={isBulkDeleting}
        profileNames={selectedProfiles}
      />
    </div>
  );
}
