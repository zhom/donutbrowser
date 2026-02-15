"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrent } from "@tauri-apps/plugin-deep-link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { CamoufoxConfigDialog } from "@/components/camoufox-config-dialog";
import { CommercialTrialModal } from "@/components/commercial-trial-modal";
import { CookieCopyDialog } from "@/components/cookie-copy-dialog";
import { CreateProfileDialog } from "@/components/create-profile-dialog";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { GroupAssignmentDialog } from "@/components/group-assignment-dialog";
import { GroupBadges } from "@/components/group-badges";
import { GroupManagementDialog } from "@/components/group-management-dialog";
import HomeHeader from "@/components/home-header";
import { ImportProfileDialog } from "@/components/import-profile-dialog";
import { IntegrationsDialog } from "@/components/integrations-dialog";
import { LaunchOnLoginDialog } from "@/components/launch-on-login-dialog";
import { PermissionDialog } from "@/components/permission-dialog";
import { ProfilesDataTable } from "@/components/profile-data-table";
import { ProfileSelectorDialog } from "@/components/profile-selector-dialog";
import { ProfileSyncDialog } from "@/components/profile-sync-dialog";
import { ProxyAssignmentDialog } from "@/components/proxy-assignment-dialog";
import { ProxyManagementDialog } from "@/components/proxy-management-dialog";
import { SettingsDialog } from "@/components/settings-dialog";
import { SyncConfigDialog } from "@/components/sync-config-dialog";
import { WayfernTermsDialog } from "@/components/wayfern-terms-dialog";
import { useAppUpdateNotifications } from "@/hooks/use-app-update-notifications";
import { useCloudAuth } from "@/hooks/use-cloud-auth";
import { useCommercialTrial } from "@/hooks/use-commercial-trial";
import { useGroupEvents } from "@/hooks/use-group-events";
import type { PermissionType } from "@/hooks/use-permissions";
import { usePermissions } from "@/hooks/use-permissions";
import { useProfileEvents } from "@/hooks/use-profile-events";
import { useProxyEvents } from "@/hooks/use-proxy-events";
import { useUpdateNotifications } from "@/hooks/use-update-notifications";
import { useVersionUpdater } from "@/hooks/use-version-updater";
import { useWayfernTerms } from "@/hooks/use-wayfern-terms";
import { showErrorToast, showSuccessToast, showToast } from "@/lib/toast-utils";
import type { BrowserProfile, CamoufoxConfig, WayfernConfig } from "@/types";

type BrowserTypeString =
  | "firefox"
  | "firefox-developer"
  | "chromium"
  | "brave"
  | "zen"
  | "camoufox"
  | "wayfern";

interface PendingUrl {
  id: string;
  url: string;
}

export default function Home() {
  // Mount global version update listener/toasts
  useVersionUpdater();

  // Use the new profile events hook for centralized profile management
  const {
    profiles,
    runningProfiles,
    isLoading: profilesLoading,
    error: profilesError,
  } = useProfileEvents();

  const {
    groups: groupsData,
    isLoading: groupsLoading,
    error: groupsError,
  } = useGroupEvents();

  const {
    storedProxies,
    isLoading: proxiesLoading,
    error: proxiesError,
  } = useProxyEvents();

  // Wayfern terms and commercial trial hooks
  const {
    termsAccepted,
    isLoading: termsLoading,
    checkTerms,
  } = useWayfernTerms();
  const {
    trialStatus,
    hasAcknowledged: trialAcknowledged,
    checkTrialStatus,
  } = useCommercialTrial();

  // Cloud auth for cross-OS unlock
  const { user: cloudUser } = useCloudAuth();
  const crossOsUnlocked =
    cloudUser?.plan !== "free" && cloudUser?.subscriptionStatus === "active";

  const [createProfileDialogOpen, setCreateProfileDialogOpen] = useState(false);
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false);
  const [integrationsDialogOpen, setIntegrationsDialogOpen] = useState(false);
  const [importProfileDialogOpen, setImportProfileDialogOpen] = useState(false);
  const [proxyManagementDialogOpen, setProxyManagementDialogOpen] =
    useState(false);
  const [camoufoxConfigDialogOpen, setCamoufoxConfigDialogOpen] =
    useState(false);
  const [groupManagementDialogOpen, setGroupManagementDialogOpen] =
    useState(false);
  const [groupAssignmentDialogOpen, setGroupAssignmentDialogOpen] =
    useState(false);
  const [proxyAssignmentDialogOpen, setProxyAssignmentDialogOpen] =
    useState(false);
  const [cookieCopyDialogOpen, setCookieCopyDialogOpen] = useState(false);
  const [selectedProfilesForCookies, setSelectedProfilesForCookies] = useState<
    string[]
  >([]);
  const [selectedGroupId, setSelectedGroupId] = useState<string>("default");
  const [selectedProfilesForGroup, setSelectedProfilesForGroup] = useState<
    string[]
  >([]);
  const [selectedProfilesForProxy, setSelectedProfilesForProxy] = useState<
    string[]
  >([]);
  const [selectedProfiles, setSelectedProfiles] = useState<string[]>([]);
  const [searchQuery, setSearchQuery] = useState<string>("");
  const [pendingUrls, setPendingUrls] = useState<PendingUrl[]>([]);
  const [currentProfileForCamoufoxConfig, setCurrentProfileForCamoufoxConfig] =
    useState<BrowserProfile | null>(null);
  const [hasCheckedStartupPrompt, setHasCheckedStartupPrompt] = useState(false);
  const [launchOnLoginDialogOpen, setLaunchOnLoginDialogOpen] = useState(false);
  const [permissionDialogOpen, setPermissionDialogOpen] = useState(false);
  const [currentPermissionType, setCurrentPermissionType] =
    useState<PermissionType>("microphone");
  const [showBulkDeleteConfirmation, setShowBulkDeleteConfirmation] =
    useState(false);
  const [isBulkDeleting, setIsBulkDeleting] = useState(false);
  const [syncConfigDialogOpen, setSyncConfigDialogOpen] = useState(false);
  const [profileSyncDialogOpen, setProfileSyncDialogOpen] = useState(false);
  const [currentProfileForSync, setCurrentProfileForSync] =
    useState<BrowserProfile | null>(null);
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

      // Also check for missing GeoIP database
      const missingGeoIP = await invoke<boolean>(
        "check_missing_geoip_database",
      );

      if (missingBinaries.length > 0 || missingGeoIP) {
        if (missingBinaries.length > 0) {
          console.log("Found missing binaries:", missingBinaries);
        }
        if (missingGeoIP) {
          console.log("Found missing GeoIP database for Camoufox");
        }

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
        let missingList = Array.from(browserMap.entries())
          .map(([browser, versions]) => `${browser}: ${versions.join(", ")}`)
          .join(", ");

        if (missingGeoIP) {
          if (missingList) {
            missingList += ", GeoIP database for Camoufox";
          } else {
            missingList = "GeoIP database for Camoufox";
          }
        }

        console.log(`Downloading missing components: ${missingList}`);

        try {
          // Download missing binaries and GeoIP database sequentially to prevent conflicts
          const downloaded = await invoke<string[]>(
            "ensure_all_binaries_exist",
          );
          if (downloaded.length > 0) {
            console.log(
              "Successfully downloaded missing components:",
              downloaded,
            );
          }
        } catch (downloadError) {
          console.error(
            "Failed to download missing components:",
            downloadError,
          );
        }
      }
    } catch (err: unknown) {
      console.error("Failed to check missing components:", err);
    }
  }, []);

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
  const updateNotifications = useUpdateNotifications();
  const { checkForUpdates, isUpdating } = updateNotifications;

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
        "should_show_launch_on_login_prompt",
      );
      if (shouldShow) {
        setLaunchOnLoginDialogOpen(true);
      }
    } catch (error) {
      console.error("Failed to check startup prompt:", error);
    } finally {
      setHasCheckedStartupPrompt(true);
    }
  }, [hasCheckedStartupPrompt]);

  // Handle profile errors from useProfileEvents hook
  useEffect(() => {
    if (profilesError) {
      showErrorToast(profilesError);
    }
  }, [profilesError]);

  // Handle group errors from useGroupEvents hook
  useEffect(() => {
    if (groupsError) {
      showErrorToast(groupsError);
    }
  }, [groupsError]);

  // Handle proxy errors from useProxyEvents hook
  useEffect(() => {
    if (proxiesError) {
      showErrorToast(proxiesError);
    }
  }, [proxiesError]);

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
        showErrorToast(
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

  const handleConfigureCamoufox = useCallback((profile: BrowserProfile) => {
    setCurrentProfileForCamoufoxConfig(profile);
    setCamoufoxConfigDialogOpen(true);
  }, []);

  const handleSaveCamoufoxConfig = useCallback(
    async (profile: BrowserProfile, config: CamoufoxConfig) => {
      try {
        await invoke("update_camoufox_config", {
          profileId: profile.id,
          config,
        });
        // No need to manually reload - useProfileEvents will handle the update
        setCamoufoxConfigDialogOpen(false);
      } catch (err: unknown) {
        console.error("Failed to update camoufox config:", err);
        showErrorToast(
          `Failed to update camoufox config: ${JSON.stringify(err)}`,
        );
        throw err;
      }
    },
    [],
  );

  const handleSaveWayfernConfig = useCallback(
    async (profile: BrowserProfile, config: WayfernConfig) => {
      try {
        await invoke("update_wayfern_config", {
          profileId: profile.id,
          config,
        });
        // No need to manually reload - useProfileEvents will handle the update
        setCamoufoxConfigDialogOpen(false);
      } catch (err: unknown) {
        console.error("Failed to update wayfern config:", err);
        showErrorToast(
          `Failed to update wayfern config: ${JSON.stringify(err)}`,
        );
        throw err;
      }
    },
    [],
  );

  const handleCreateProfile = useCallback(
    async (profileData: {
      name: string;
      browserStr: BrowserTypeString;
      version: string;
      releaseType: string;
      proxyId?: string;
      camoufoxConfig?: CamoufoxConfig;
      wayfernConfig?: WayfernConfig;
      groupId?: string;
    }) => {
      try {
        await invoke<BrowserProfile>("create_browser_profile_new", {
          name: profileData.name,
          browserStr: profileData.browserStr,
          version: profileData.version,
          releaseType: profileData.releaseType,
          proxyId: profileData.proxyId,
          camoufoxConfig: profileData.camoufoxConfig,
          wayfernConfig: profileData.wayfernConfig,
          groupId:
            profileData.groupId ||
            (selectedGroupId !== "default" ? selectedGroupId : undefined),
        });

        // No need to manually reload - useProfileEvents will handle the update
      } catch (error) {
        showErrorToast(
          `Failed to create profile: ${
            error instanceof Error ? error.message : String(error)
          }`,
        );
        throw error;
      }
    },
    [selectedGroupId],
  );

  const launchProfile = useCallback(async (profile: BrowserProfile) => {
    console.log("Starting launch for profile:", profile.name);

    try {
      const result = await invoke<BrowserProfile>("launch_browser_profile", {
        profile,
      });
      console.log("Successfully launched profile:", result.name);
    } catch (err: unknown) {
      console.error("Failed to launch browser:", err);
      const errorMessage = err instanceof Error ? err.message : String(err);
      showErrorToast(`Failed to launch browser: ${errorMessage}`);
      // Re-throw the error so the table component can handle loading state cleanup
      throw err;
    }
  }, []);

  const handleCloneProfile = useCallback(async (profile: BrowserProfile) => {
    try {
      await invoke<BrowserProfile>("clone_profile", {
        profileId: profile.id,
      });
    } catch (err: unknown) {
      console.error("Failed to clone profile:", err);
      const errorMessage = err instanceof Error ? err.message : String(err);
      showErrorToast(`Failed to clone profile: ${errorMessage}`);
    }
  }, []);

  const handleDeleteProfile = useCallback(async (profile: BrowserProfile) => {
    console.log("Attempting to delete profile:", profile.name);

    try {
      // First check if the browser is running for this profile
      const isRunning = await invoke<boolean>("check_browser_status", {
        profile,
      });

      if (isRunning) {
        showErrorToast(
          "Cannot delete profile while browser is running. Please stop the browser first.",
        );
        return;
      }

      // Attempt to delete the profile
      await invoke("delete_profile", { profileId: profile.id });
      console.log("Profile deletion command completed successfully");

      // No need to manually reload - useProfileEvents will handle the update
      console.log("Profile deleted successfully");
    } catch (err: unknown) {
      console.error("Failed to delete profile:", err);
      const errorMessage = err instanceof Error ? err.message : String(err);
      showErrorToast(`Failed to delete profile: ${errorMessage}`);
    }
  }, []);

  const handleRenameProfile = useCallback(
    async (profileId: string, newName: string) => {
      try {
        await invoke("rename_profile", { profileId, newName });
        // No need to manually reload - useProfileEvents will handle the update
      } catch (err: unknown) {
        console.error("Failed to rename profile:", err);
        showErrorToast(`Failed to rename profile: ${JSON.stringify(err)}`);
        throw err;
      }
    },
    [],
  );

  const handleKillProfile = useCallback(async (profile: BrowserProfile) => {
    console.log("Starting kill for profile:", profile.name);

    try {
      await invoke("kill_browser_profile", { profile });
      console.log("Successfully killed profile:", profile.name);
      // No need to manually reload - useProfileEvents will handle the update
    } catch (err: unknown) {
      console.error("Failed to kill browser:", err);
      const errorMessage = err instanceof Error ? err.message : String(err);
      showErrorToast(`Failed to kill browser: ${errorMessage}`);
      // Re-throw the error so the table component can handle loading state cleanup
      throw err;
    }
  }, []);

  const handleDeleteSelectedProfiles = useCallback(
    async (profileIds: string[]) => {
      try {
        await invoke("delete_selected_profiles", { profileIds });
        // No need to manually reload - useProfileEvents will handle the update
      } catch (err: unknown) {
        console.error("Failed to delete selected profiles:", err);
        showErrorToast(
          `Failed to delete selected profiles: ${JSON.stringify(err)}`,
        );
      }
    },
    [],
  );

  const handleAssignProfilesToGroup = useCallback((profileIds: string[]) => {
    setSelectedProfilesForGroup(profileIds);
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
        profileIds: selectedProfiles,
      });
      // No need to manually reload - useProfileEvents will handle the update
      setSelectedProfiles([]);
      setShowBulkDeleteConfirmation(false);
    } catch (error) {
      console.error("Failed to delete selected profiles:", error);
      showErrorToast(
        `Failed to delete selected profiles: ${JSON.stringify(error)}`,
      );
    } finally {
      setIsBulkDeleting(false);
    }
  }, [selectedProfiles]);

  const handleBulkGroupAssignment = useCallback(() => {
    if (selectedProfiles.length === 0) return;
    handleAssignProfilesToGroup(selectedProfiles);
    setSelectedProfiles([]);
  }, [selectedProfiles, handleAssignProfilesToGroup]);

  const handleAssignProfilesToProxy = useCallback((profileIds: string[]) => {
    setSelectedProfilesForProxy(profileIds);
    setProxyAssignmentDialogOpen(true);
  }, []);

  const handleBulkProxyAssignment = useCallback(() => {
    if (selectedProfiles.length === 0) return;
    handleAssignProfilesToProxy(selectedProfiles);
    setSelectedProfiles([]);
  }, [selectedProfiles, handleAssignProfilesToProxy]);

  const handleBulkCopyCookies = useCallback(() => {
    if (selectedProfiles.length === 0) return;
    const eligibleProfiles = profiles.filter(
      (p) =>
        selectedProfiles.includes(p.id) &&
        (p.browser === "wayfern" || p.browser === "camoufox"),
    );
    if (eligibleProfiles.length === 0) {
      showErrorToast(
        "Cookie copy only works with Wayfern and Camoufox profiles",
      );
      return;
    }
    setSelectedProfilesForCookies(eligibleProfiles.map((p) => p.id));
    setCookieCopyDialogOpen(true);
  }, [selectedProfiles, profiles]);

  const handleCopyCookiesToProfile = useCallback((profile: BrowserProfile) => {
    setSelectedProfilesForCookies([profile.id]);
    setCookieCopyDialogOpen(true);
  }, []);

  const handleGroupAssignmentComplete = useCallback(async () => {
    // No need to manually reload - useProfileEvents will handle the update
    setGroupAssignmentDialogOpen(false);
    setSelectedProfilesForGroup([]);
  }, []);

  const handleProxyAssignmentComplete = useCallback(async () => {
    // No need to manually reload - useProfileEvents will handle the update
    setProxyAssignmentDialogOpen(false);
    setSelectedProfilesForProxy([]);
  }, []);

  const handleGroupManagementComplete = useCallback(async () => {
    // No need to manually reload - useProfileEvents will handle the update
  }, []);

  const handleOpenProfileSyncDialog = useCallback((profile: BrowserProfile) => {
    setCurrentProfileForSync(profile);
    setProfileSyncDialogOpen(true);
  }, []);

  const handleToggleProfileSync = useCallback(
    async (profile: BrowserProfile) => {
      try {
        await invoke("set_profile_sync_enabled", {
          profileId: profile.id,
          enabled: !profile.sync_enabled,
        });
        showSuccessToast(
          profile.sync_enabled ? "Sync disabled" : "Sync enabled",
          {
            description: profile.sync_enabled
              ? "Profile sync has been disabled"
              : "Profile sync has been enabled",
          },
        );
      } catch (error) {
        console.error("Failed to toggle sync:", error);
        showErrorToast("Failed to update sync settings");
      }
    },
    [],
  );

  useEffect(() => {
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

    // Check for missing binaries after initial profile load
    if (!profilesLoading && profiles.length > 0) {
      void checkMissingBinaries();
    }

    // Proactively download Wayfern and Camoufox if not already available
    if (!profilesLoading) {
      void invoke("ensure_active_browsers_downloaded").catch((err: unknown) => {
        console.error("Failed to auto-download browsers:", err);
      });
    }

    return () => {
      clearInterval(updateInterval);
      if (cleanup) {
        cleanup();
      }
    };
  }, [
    checkForUpdates,
    checkStartupPrompt,
    listenForUrlEvents,
    checkCurrentUrl,
    checkMissingBinaries,
    profilesLoading,
    profiles.length,
  ]);

  // Show deprecation warning for unsupported profiles (with names)
  useEffect(() => {
    if (profiles.length === 0) return;

    const deprecatedProfiles = profiles.filter(
      (p) => p.release_type === "nightly" && p.browser !== "firefox-developer",
    );

    if (deprecatedProfiles.length > 0) {
      const deprecatedNames = deprecatedProfiles.map((p) => p.name).join(", ");

      // Use a stable id to avoid duplicate toasts on re-renders
      showToast({
        id: "deprecated-profiles-warning",
        type: "error",
        title: "Some profiles will be deprecated soon",
        description: `The following profiles will be deprecated soon: ${deprecatedNames}. Nightly profiles (except Firefox Developers Edition) will be removed in upcoming versions. Please check GitHub for migration instructions.`,
        duration: 15000,
        action: {
          label: "Learn more",
          onClick: () => {
            const event = new CustomEvent("url-open-request", {
              detail: "https://github.com/zhom/donutbrowser/discussions/66",
            });
            window.dispatchEvent(event);
          },
        },
      });
    }
  }, [profiles]);

  // Show warning for non-wayfern/camoufox profiles (support ending March 15, 2026)
  useEffect(() => {
    if (profiles.length === 0) return;

    const unsupportedProfiles = profiles.filter(
      (p) => p.browser !== "wayfern" && p.browser !== "camoufox",
    );

    if (unsupportedProfiles.length > 0) {
      const unsupportedNames = unsupportedProfiles
        .map((p) => p.name)
        .join(", ");

      showToast({
        id: "browser-support-ending-warning",
        type: "error",
        title: "Browser support ending soon",
        description: `Support for the following profiles will be removed on March 15, 2026: ${unsupportedNames}. Please migrate to Wayfern or Camoufox profiles.`,
        duration: 15000,
        action: {
          label: "Learn more",
          onClick: () => {
            const event = new CustomEvent("url-open-request", {
              detail: "https://github.com/zhom/donutbrowser/discussions",
            });
            window.dispatchEvent(event);
          },
        },
      });
    }
  }, [profiles]);

  // Re-check Wayfern terms when a browser download completes
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    const setup = async () => {
      unlisten = await listen<{ stage: string }>(
        "download-progress",
        (event) => {
          if (event.payload.stage === "completed") {
            void checkTerms();
          }
        },
      );
    };
    void setup();
    return () => {
      if (unlisten) unlisten();
    };
  }, [checkTerms]);

  // Check permissions when they are initialized
  useEffect(() => {
    if (isInitialized) {
      void checkAllPermissions();
    }
  }, [isInitialized, checkAllPermissions]);

  // Filter data by selected group and search query
  const filteredProfiles = useMemo(() => {
    let filtered = profiles;

    // Filter by group
    if (!selectedGroupId || selectedGroupId === "default") {
      filtered = profiles.filter((profile) => !profile.group_id);
    } else {
      filtered = profiles.filter(
        (profile) => profile.group_id === selectedGroupId,
      );
    }

    // Filter by search query
    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase().trim();
      filtered = filtered.filter((profile) => {
        // Search in profile name
        if (profile.name.toLowerCase().includes(query)) return true;

        // Search in note
        if (profile.note?.toLowerCase().includes(query)) return true;

        // Search in tags
        if (profile.tags?.some((tag) => tag.toLowerCase().includes(query)))
          return true;

        return false;
      });
    }

    return filtered;
  }, [profiles, selectedGroupId, searchQuery]);

  // Update loading states
  const isLoading = profilesLoading || groupsLoading || proxiesLoading;

  return (
    <div className="grid items-center justify-items-center min-h-screen gap-8 font-(family-name:--font-geist-sans) bg-background">
      <main className="flex flex-col items-center w-full max-w-3xl">
        <div className="w-full">
          <HomeHeader
            onCreateProfileDialogOpen={setCreateProfileDialogOpen}
            onGroupManagementDialogOpen={setGroupManagementDialogOpen}
            onImportProfileDialogOpen={setImportProfileDialogOpen}
            onProxyManagementDialogOpen={setProxyManagementDialogOpen}
            onSettingsDialogOpen={setSettingsDialogOpen}
            onSyncConfigDialogOpen={setSyncConfigDialogOpen}
            onIntegrationsDialogOpen={setIntegrationsDialogOpen}
            searchQuery={searchQuery}
            onSearchQueryChange={setSearchQuery}
          />
        </div>
        <div className="w-full mt-2.5">
          <GroupBadges
            selectedGroupId={selectedGroupId}
            onGroupSelect={handleSelectGroup}
            groups={groupsData}
            isLoading={isLoading}
          />
          <ProfilesDataTable
            profiles={filteredProfiles}
            onLaunchProfile={launchProfile}
            onKillProfile={handleKillProfile}
            onCloneProfile={handleCloneProfile}
            onDeleteProfile={handleDeleteProfile}
            onRenameProfile={handleRenameProfile}
            onConfigureCamoufox={handleConfigureCamoufox}
            onCopyCookiesToProfile={handleCopyCookiesToProfile}
            runningProfiles={runningProfiles}
            isUpdating={isUpdating}
            onDeleteSelectedProfiles={handleDeleteSelectedProfiles}
            onAssignProfilesToGroup={handleAssignProfilesToGroup}
            selectedGroupId={selectedGroupId}
            selectedProfiles={selectedProfiles}
            onSelectedProfilesChange={setSelectedProfiles}
            onBulkDelete={handleBulkDelete}
            onBulkGroupAssignment={handleBulkGroupAssignment}
            onBulkProxyAssignment={handleBulkProxyAssignment}
            onBulkCopyCookies={handleBulkCopyCookies}
            onOpenProfileSyncDialog={handleOpenProfileSyncDialog}
            onToggleProfileSync={handleToggleProfileSync}
          />
        </div>
      </main>

      <CreateProfileDialog
        isOpen={createProfileDialogOpen}
        onClose={() => {
          setCreateProfileDialogOpen(false);
        }}
        onCreateProfile={handleCreateProfile}
        selectedGroupId={selectedGroupId}
        crossOsUnlocked={crossOsUnlocked}
      />

      <SettingsDialog
        isOpen={settingsDialogOpen}
        onClose={() => {
          setSettingsDialogOpen(false);
        }}
        onIntegrationsOpen={() => {
          setSettingsDialogOpen(false);
          setIntegrationsDialogOpen(true);
        }}
      />

      <IntegrationsDialog
        isOpen={integrationsDialogOpen}
        onClose={() => {
          setIntegrationsDialogOpen(false);
        }}
      />

      <ImportProfileDialog
        isOpen={importProfileDialogOpen}
        onClose={() => {
          setImportProfileDialogOpen(false);
        }}
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
        onSaveWayfern={handleSaveWayfernConfig}
        isRunning={
          currentProfileForCamoufoxConfig
            ? runningProfiles.has(currentProfileForCamoufoxConfig.id)
            : false
        }
        crossOsUnlocked={crossOsUnlocked}
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
        profiles={profiles}
      />

      <ProxyAssignmentDialog
        isOpen={proxyAssignmentDialogOpen}
        onClose={() => {
          setProxyAssignmentDialogOpen(false);
        }}
        selectedProfiles={selectedProfilesForProxy}
        onAssignmentComplete={handleProxyAssignmentComplete}
        profiles={profiles}
        storedProxies={storedProxies}
      />

      <CookieCopyDialog
        isOpen={cookieCopyDialogOpen}
        onClose={() => {
          setCookieCopyDialogOpen(false);
          setSelectedProfilesForCookies([]);
        }}
        selectedProfiles={selectedProfilesForCookies}
        profiles={profiles}
        runningProfiles={runningProfiles}
        onCopyComplete={() => setSelectedProfilesForCookies([])}
      />

      <DeleteConfirmationDialog
        isOpen={showBulkDeleteConfirmation}
        onClose={() => setShowBulkDeleteConfirmation(false)}
        onConfirm={confirmBulkDelete}
        title="Delete Selected Profiles"
        description={`This action cannot be undone. This will permanently delete ${selectedProfiles.length} profile${selectedProfiles.length !== 1 ? "s" : ""} and all associated data.`}
        confirmButtonText={`Delete ${selectedProfiles.length} Profile${selectedProfiles.length !== 1 ? "s" : ""}`}
        isLoading={isBulkDeleting}
        profileIds={selectedProfiles}
        profiles={profiles.map((p) => ({ id: p.id, name: p.name }))}
      />

      <SyncConfigDialog
        isOpen={syncConfigDialogOpen}
        onClose={() => setSyncConfigDialogOpen(false)}
      />

      <ProfileSyncDialog
        isOpen={profileSyncDialogOpen}
        onClose={() => {
          setProfileSyncDialogOpen(false);
          setCurrentProfileForSync(null);
        }}
        profile={currentProfileForSync}
        onSyncConfigOpen={() => setSyncConfigDialogOpen(true)}
      />

      {/* Wayfern Terms and Conditions Dialog - shown if terms not accepted */}
      <WayfernTermsDialog
        isOpen={!termsLoading && termsAccepted === false}
        onAccepted={checkTerms}
      />

      {/* Commercial Trial Modal - shown once when trial expires */}
      <CommercialTrialModal
        isOpen={
          !termsLoading &&
          termsAccepted === true &&
          trialStatus?.type === "Expired" &&
          !trialAcknowledged
        }
        onClose={checkTrialStatus}
      />

      {/* Launch on Login Dialog - shown on every startup until enabled or declined */}
      <LaunchOnLoginDialog
        isOpen={launchOnLoginDialogOpen}
        onClose={() => setLaunchOnLoginDialogOpen(false)}
      />
    </div>
  );
}
