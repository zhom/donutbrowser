"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import { LuCheck, LuChevronsUpDown } from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import { ProxyFormDialog } from "@/components/proxy-form-dialog";
import { SharedCamoufoxConfigForm } from "@/components/shared-camoufox-config-form";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
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
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent } from "@/components/ui/tabs";
import { WayfernConfigForm } from "@/components/wayfern-config-form";
import { useBrowserDownload } from "@/hooks/use-browser-download";
import { useProxyEvents } from "@/hooks/use-proxy-events";
import { useVpnEvents } from "@/hooks/use-vpn-events";
import { getBrowserIcon } from "@/lib/browser-utils";
import { cn } from "@/lib/utils";
import type {
  BrowserReleaseTypes,
  CamoufoxConfig,
  CamoufoxOS,
  WayfernConfig,
  WayfernOS,
} from "@/types";

const getCurrentOS = (): CamoufoxOS => {
  if (typeof navigator === "undefined") return "linux";
  const platform = navigator.platform.toLowerCase();
  if (platform.includes("win")) return "windows";
  if (platform.includes("mac")) return "macos";
  return "linux";
};

import { RippleButton } from "./ui/ripple";

type BrowserTypeString = "camoufox" | "wayfern";

interface CreateProfileDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onCreateProfile: (profileData: {
    name: string;
    browserStr: BrowserTypeString;
    version: string;
    releaseType: string;
    proxyId?: string;
    vpnId?: string;
    camoufoxConfig?: CamoufoxConfig;
    wayfernConfig?: WayfernConfig;
    groupId?: string;
    extensionGroupId?: string;
    ephemeral?: boolean;
    dnsBlocklist?: string;
    launchHook?: string;
  }) => Promise<void>;
  selectedGroupId?: string;
  crossOsUnlocked?: boolean;
}

interface BrowserOption {
  value: BrowserTypeString;
  label: string;
}

const browserOptions: BrowserOption[] = [
  {
    value: "camoufox",
    label: "Camoufox",
  },
  {
    value: "wayfern",
    label: "Wayfern",
  },
];

export function CreateProfileDialog({
  isOpen,
  onClose,
  onCreateProfile,
  selectedGroupId,
  crossOsUnlocked = false,
}: CreateProfileDialogProps) {
  const { t } = useTranslation();
  const [profileName, setProfileName] = useState("");
  const [currentStep, setCurrentStep] = useState<
    "browser-selection" | "browser-config"
  >("browser-selection");
  const [activeTab, setActiveTab] = useState("anti-detect");

  // Browser selection states
  const [selectedBrowser, setSelectedBrowser] =
    useState<BrowserTypeString | null>(null);
  const [selectedProxyId, setSelectedProxyId] = useState<string>();
  const [proxyPopoverOpen, setProxyPopoverOpen] = useState(false);
  const [dnsBlocklist, setDnsBlocklist] = useState<string>("");
  const [launchHook, setLaunchHook] = useState("");

  // Camoufox anti-detect states
  const [camoufoxConfig, setCamoufoxConfig] = useState<CamoufoxConfig>(() => ({
    geoip: true, // Default to automatic geoip
    os: getCurrentOS(), // Default to current OS
  }));

  // Wayfern anti-detect states
  const [wayfernConfig, setWayfernConfig] = useState<WayfernConfig>(() => ({
    os: getCurrentOS() as WayfernOS, // Default to current OS
  }));

  // Handle browser selection from the initial screen
  const handleBrowserSelect = (browser: BrowserTypeString) => {
    setSelectedBrowser(browser);
    setCurrentStep("browser-config");
  };

  // Handle back button
  const handleBack = () => {
    setCurrentStep("browser-selection");
    setSelectedBrowser(null);
    setProfileName("");
    setSelectedProxyId(undefined);
    setLaunchHook("");
  };

  const handleTabChange = (value: string) => {
    setActiveTab(value);
    setCurrentStep("browser-selection");
    setSelectedBrowser(null);
    setProfileName("");
    setSelectedProxyId(undefined);
    setLaunchHook("");
  };

  const [supportedBrowsers, setSupportedBrowsers] = useState<string[]>([]);
  const { storedProxies } = useProxyEvents();
  const { vpnConfigs } = useVpnEvents();
  const [showProxyForm, setShowProxyForm] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [ephemeral, setEphemeral] = useState(false);
  const [selectedExtensionGroupId, setSelectedExtensionGroupId] =
    useState<string>();
  const [extensionGroups, setExtensionGroups] = useState<
    { id: string; name: string; extension_ids: string[] }[]
  >([]);

  useEffect(() => {
    if (isOpen) {
      void invoke<{ id: string; name: string; extension_ids: string[] }[]>(
        "list_extension_groups",
      )
        .then(setExtensionGroups)
        .catch(() => {
          setExtensionGroups([]);
        });
    }
  }, [isOpen]);
  const [releaseTypes, setReleaseTypes] = useState<BrowserReleaseTypes>();
  const [isLoadingReleaseTypes, setIsLoadingReleaseTypes] = useState(false);
  const [releaseTypesError, setReleaseTypesError] = useState<string | null>(
    null,
  );
  const loadingBrowserRef = useRef<string | null>(null);

  // Use the browser download hook
  const {
    isBrowserDownloading,
    downloadBrowser,
    loadDownloadedVersions,
    isVersionDownloaded,
    downloadedVersionsMap,
  } = useBrowserDownload();

  const loadSupportedBrowsers = useCallback(async () => {
    try {
      const browsers = await invoke<string[]>("get_supported_browsers");
      setSupportedBrowsers(browsers);
    } catch (error) {
      console.error("Failed to load supported browsers:", error);
    }
  }, []);

  const checkAndDownloadGeoIPDatabase = useCallback(async () => {
    try {
      const isAvailable = await invoke<boolean>("is_geoip_database_available");
      if (!isAvailable) {
        console.log("GeoIP database not available, downloading...");
        await invoke("download_geoip_database");
        console.log("GeoIP database downloaded successfully");
      }
    } catch (error) {
      console.error("Failed to check/download GeoIP database:", error);
      // Don't show error to user as this is not critical for profile creation
    }
  }, []);

  const loadReleaseTypes = useCallback(
    async (browser: string) => {
      // Set loading state
      loadingBrowserRef.current = browser;
      setIsLoadingReleaseTypes(true);
      setReleaseTypesError(null);

      try {
        const rawReleaseTypes = await invoke<BrowserReleaseTypes>(
          "get_browser_release_types",
          { browserStr: browser },
        );

        await loadDownloadedVersions(browser);

        // Only update state if this browser is still the one we're loading
        if (loadingBrowserRef.current === browser) {
          const filtered: BrowserReleaseTypes = {};
          if (rawReleaseTypes.stable) filtered.stable = rawReleaseTypes.stable;
          setReleaseTypes(filtered);
          setReleaseTypesError(null);
        }
      } catch (error) {
        console.error(`Failed to load release types for ${browser}:`, error);

        // Fallback: still load downloaded versions and derive release type from them if possible
        try {
          const downloaded = await loadDownloadedVersions(browser);
          if (loadingBrowserRef.current === browser && downloaded.length > 0) {
            const latest = downloaded[0];
            const fallback: BrowserReleaseTypes = {};
            fallback.stable = latest;
            setReleaseTypes(fallback);
            setReleaseTypesError(null);
          } else if (loadingBrowserRef.current === browser) {
            // No downloaded versions and API failed - show error
            setReleaseTypesError(
              "Failed to fetch browser versions. Please check your internet connection and try again.",
            );
          }
        } catch (e) {
          console.error(
            `Failed to load downloaded versions for ${browser}:`,
            e,
          );
          if (loadingBrowserRef.current === browser) {
            setReleaseTypesError(
              "Failed to fetch browser versions. Please check your internet connection and try again.",
            );
          }
        }
      } finally {
        // Clear loading state only if we're still loading this browser
        if (loadingBrowserRef.current === browser) {
          loadingBrowserRef.current = null;
          setIsLoadingReleaseTypes(false);
        }
      }
    },
    [loadDownloadedVersions],
  );

  // Load data when dialog opens
  useEffect(() => {
    if (isOpen) {
      void loadSupportedBrowsers();
      // Load release types when a browser is selected
      if (selectedBrowser) {
        void loadReleaseTypes(selectedBrowser);
      }
      // Check and download GeoIP database if needed for Camoufox or Wayfern
      if (selectedBrowser === "camoufox" || selectedBrowser === "wayfern") {
        void checkAndDownloadGeoIPDatabase();
      }
    }
  }, [
    isOpen,
    loadSupportedBrowsers,
    loadReleaseTypes,
    checkAndDownloadGeoIPDatabase,
    selectedBrowser,
  ]);

  // Load release types when browser selection changes
  useEffect(() => {
    if (selectedBrowser) {
      // Cancel any previous loading
      loadingBrowserRef.current = null;
      // Clear previous release types immediately to prevent showing stale data
      setReleaseTypes({});
      void loadReleaseTypes(selectedBrowser);
    }
  }, [selectedBrowser, loadReleaseTypes]);

  // Helper function to get the best available version respecting rules
  const getBestAvailableVersion = useCallback(
    (_browserType?: string) => {
      if (!releaseTypes) return null;

      if (releaseTypes.stable) {
        return { version: releaseTypes.stable, releaseType: "stable" as const };
      }
      return null;
    },
    [releaseTypes],
  );

  const getCreatableVersion = useCallback(
    (browserType?: string) => {
      const bestVersion = getBestAvailableVersion(browserType);
      if (bestVersion && isVersionDownloaded(bestVersion.version)) {
        return bestVersion;
      }
      const browserDownloaded = downloadedVersionsMap[browserType ?? ""] ?? [];
      if (browserDownloaded.length > 0) {
        const fallbackVersion = browserDownloaded[0];
        return {
          version: fallbackVersion,
          releaseType: "stable" as const,
        };
      }
      return null;
    },
    [getBestAvailableVersion, isVersionDownloaded, downloadedVersionsMap],
  );

  const handleDownload = async (browserStr: string) => {
    const bestVersion = getBestAvailableVersion(browserStr);

    if (!bestVersion) {
      console.error("No version available for download");
      return;
    }

    try {
      await downloadBrowser(browserStr, bestVersion.version);
    } catch (error) {
      console.error("Failed to download browser:", error);
    }
  };

  const handleCreate = async () => {
    if (!profileName.trim()) return;

    setIsCreating(true);

    const isVpnSelection = selectedProxyId?.startsWith("vpn-") ?? false;
    const resolvedProxyId = isVpnSelection ? undefined : selectedProxyId;
    const resolvedVpnId =
      isVpnSelection && selectedProxyId ? selectedProxyId.slice(4) : undefined;
    try {
      if (activeTab === "anti-detect") {
        // Anti-detect browser - check if Wayfern or Camoufox is selected
        if (selectedBrowser === "wayfern") {
          const bestWayfernVersion = getCreatableVersion("wayfern");
          if (!bestWayfernVersion) {
            console.error("No Wayfern version available");
            return;
          }

          // The fingerprint will be generated at launch time by the Rust backend
          const finalWayfernConfig = { ...wayfernConfig };

          await onCreateProfile({
            name: profileName.trim(),
            browserStr: "wayfern" as BrowserTypeString,
            version: bestWayfernVersion.version,
            releaseType: bestWayfernVersion.releaseType,
            proxyId: resolvedProxyId,
            vpnId: resolvedVpnId,
            wayfernConfig: finalWayfernConfig,
            groupId:
              selectedGroupId !== "default" ? selectedGroupId : undefined,
            extensionGroupId: selectedExtensionGroupId,
            ephemeral,
            dnsBlocklist: dnsBlocklist || undefined,
            launchHook: launchHook.trim() || undefined,
          });
        } else {
          // Default to Camoufox
          const bestCamoufoxVersion = getCreatableVersion("camoufox");
          if (!bestCamoufoxVersion) {
            console.error("No Camoufox version available");
            return;
          }

          // The fingerprint will be generated at launch time by the Rust backend
          // We don't need to generate it here during profile creation
          const finalCamoufoxConfig = { ...camoufoxConfig };

          await onCreateProfile({
            name: profileName.trim(),
            browserStr: "camoufox" as BrowserTypeString,
            version: bestCamoufoxVersion.version,
            releaseType: bestCamoufoxVersion.releaseType,
            proxyId: resolvedProxyId,
            vpnId: resolvedVpnId,
            camoufoxConfig: finalCamoufoxConfig,
            groupId:
              selectedGroupId !== "default" ? selectedGroupId : undefined,
            extensionGroupId: selectedExtensionGroupId,
            ephemeral,
            dnsBlocklist: dnsBlocklist || undefined,
            launchHook: launchHook.trim() || undefined,
          });
        }
      } else {
        // Regular browser
        if (!selectedBrowser) {
          console.error("Missing required browser selection");
          return;
        }

        // Use the best available version (stable preferred, nightly as fallback)
        const bestVersion = getCreatableVersion(selectedBrowser);
        if (!bestVersion) {
          console.error("No version available");
          return;
        }

        await onCreateProfile({
          name: profileName.trim(),
          browserStr: selectedBrowser,
          version: bestVersion.version,
          releaseType: bestVersion.releaseType,
          proxyId: selectedProxyId,
          groupId: selectedGroupId !== "default" ? selectedGroupId : undefined,
          dnsBlocklist: dnsBlocklist || undefined,
          launchHook: launchHook.trim() || undefined,
        });
      }

      handleClose();
    } catch (error) {
      console.error("Failed to create profile:", error);
    } finally {
      setIsCreating(false);
    }
  };

  const handleClose = () => {
    // Cancel any ongoing loading
    loadingBrowserRef.current = null;

    // Reset all states
    setProfileName("");
    setCurrentStep("browser-selection");
    setActiveTab("anti-detect");
    setSelectedBrowser(null);
    setSelectedProxyId(undefined);
    setLaunchHook("");
    setReleaseTypes({});
    setIsLoadingReleaseTypes(false);
    setReleaseTypesError(null);
    setCamoufoxConfig({
      geoip: true, // Reset to automatic geoip
      os: getCurrentOS(), // Reset to current OS
    });
    setWayfernConfig({
      os: getCurrentOS() as WayfernOS, // Reset to current OS
    });
    setEphemeral(false);
    onClose();
  };

  const updateCamoufoxConfig = (key: keyof CamoufoxConfig, value: unknown) => {
    setCamoufoxConfig((prev) => ({ ...prev, [key]: value }));
  };

  const updateWayfernConfig = (key: keyof WayfernConfig, value: unknown) => {
    setWayfernConfig((prev) => ({ ...prev, [key]: value }));
  };

  // Check if browser version is downloaded and available
  const isBrowserVersionAvailable = useCallback(
    (browserStr: string) => {
      const bestVersion = getBestAvailableVersion(browserStr);
      return bestVersion && isVersionDownloaded(bestVersion.version);
    },
    [isVersionDownloaded, getBestAvailableVersion],
  );

  // Check if browser is currently downloading
  const isBrowserCurrentlyDownloading = useCallback(
    (browserStr: string) => {
      return isBrowserDownloading(browserStr);
    },
    [isBrowserDownloading],
  );

  const isCreateDisabled = useMemo(() => {
    if (!profileName.trim()) return true;
    if (!selectedBrowser) return true;
    if (isBrowserCurrentlyDownloading(selectedBrowser)) return true;
    if (!getCreatableVersion(selectedBrowser)) return true;

    return false;
  }, [
    profileName,
    selectedBrowser,
    isBrowserCurrentlyDownloading,
    getCreatableVersion,
  ]);

  // Filter supported browsers for regular browsers
  const regularBrowsers = browserOptions.filter((browser) =>
    supportedBrowsers.includes(browser.value),
  );

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="w-full max-h-[90vh] flex flex-col">
        <DialogHeader className="flex-shrink-0">
          <DialogTitle>
            {currentStep === "browser-selection"
              ? t("createProfile.title")
              : t("createProfile.configureTitle", {
                  browser:
                    selectedBrowser === "wayfern"
                      ? t("createProfile.chromiumLabel")
                      : t("createProfile.firefoxLabel"),
                })}
          </DialogTitle>
        </DialogHeader>

        <Tabs
          value={activeTab}
          onValueChange={handleTabChange}
          className="flex flex-col flex-1 w-full min-h-0"
        >
          {/* Tab list hidden - only anti-detect browsers are supported */}

          <ScrollArea className="overflow-y-auto flex-1">
            <div className="flex flex-col justify-center items-center w-full">
              <div className="py-4 space-y-6 w-full max-w-md">
                {currentStep === "browser-selection" ? (
                  <>
                    <TabsContent value="anti-detect" className="mt-0 space-y-6">
                      {/* Anti-Detect Browser Selection */}
                      <div className="space-y-3 pt-8">
                        {/* Wayfern (Chromium) - First */}
                        <Button
                          onClick={() => {
                            handleBrowserSelect("wayfern");
                          }}
                          className="flex gap-3 justify-start items-center p-4 w-full h-16 border-2 transition-colors hover:border-primary/50"
                          variant="outline"
                        >
                          <div className="flex justify-center items-center w-8 h-8">
                            {(() => {
                              const IconComponent = getBrowserIcon("wayfern");
                              return IconComponent ? (
                                <IconComponent className="w-6 h-6" />
                              ) : null;
                            })()}
                          </div>
                          <div className="text-left">
                            <div className="font-medium">
                              {t("createProfile.chromiumLabel")}
                            </div>
                            <div className="text-sm text-muted-foreground">
                              {t("createProfile.chromiumSubtitle")}
                            </div>
                          </div>
                        </Button>

                        {/* Camoufox (Firefox) - Second */}
                        <Button
                          onClick={() => {
                            handleBrowserSelect("camoufox");
                          }}
                          className="flex gap-3 justify-start items-center p-4 w-full h-16 border-2 transition-colors hover:border-primary/50"
                          variant="outline"
                        >
                          <div className="flex justify-center items-center w-8 h-8">
                            {(() => {
                              const IconComponent = getBrowserIcon("camoufox");
                              return IconComponent ? (
                                <IconComponent className="w-6 h-6" />
                              ) : null;
                            })()}
                          </div>
                          <div className="text-left">
                            <div className="font-medium">
                              {t("createProfile.firefoxLabel")}
                            </div>
                            <div className="text-sm text-muted-foreground">
                              {t("createProfile.firefoxSubtitle")}
                            </div>
                          </div>
                        </Button>
                      </div>
                    </TabsContent>

                    <TabsContent value="regular" className="mt-0 space-y-6">
                      {/* Regular Browser Selection */}
                      <div className="space-y-6">
                        <div className="text-center">
                          <h3 className="text-lg font-medium">
                            Regular Browsers
                          </h3>
                          <p className="mt-2 text-sm text-muted-foreground">
                            Choose from supported regular browsers
                          </p>
                        </div>

                        <div className="space-y-3">
                          {regularBrowsers.map((browser) => {
                            if (browser.value === "camoufox") return null; // Skip camoufox as it's handled in anti-detect tab
                            const IconComponent = getBrowserIcon(browser.value);
                            return (
                              <Button
                                key={browser.value}
                                onClick={() => {
                                  handleBrowserSelect(browser.value);
                                }}
                                className="flex gap-3 justify-start items-center p-4 w-full h-16 border-2 transition-colors hover:border-primary/50"
                                variant="outline"
                              >
                                <div className="flex justify-center items-center w-8 h-8">
                                  {IconComponent && (
                                    <IconComponent className="w-6 h-6" />
                                  )}
                                </div>
                                <div className="text-left">
                                  <div className="font-medium">
                                    {browser.label}
                                  </div>
                                  <div className="text-sm text-muted-foreground">
                                    Regular Browser
                                  </div>
                                </div>
                              </Button>
                            );
                          })}
                        </div>
                      </div>
                    </TabsContent>
                  </>
                ) : (
                  <>
                    <TabsContent value="anti-detect" className="mt-0">
                      {/* Anti-Detect Configuration */}
                      <div className="space-y-6">
                        {/* Profile Name */}
                        <div className="space-y-2">
                          <Label htmlFor="profile-name">Profile Name</Label>
                          <Input
                            id="profile-name"
                            value={profileName}
                            onChange={(e) => {
                              setProfileName(e.target.value);
                            }}
                            onKeyDown={(e) => {
                              if (
                                e.key === "Enter" &&
                                !isCreateDisabled &&
                                !isCreating
                              ) {
                                void handleCreate();
                              }
                            }}
                            placeholder="Enter profile name"
                          />
                        </div>

                        {/* Ephemeral Option */}
                        <div className="space-y-3 p-4 border rounded-lg bg-muted/30">
                          <div className="flex items-center space-x-2">
                            <Checkbox
                              id="ephemeral"
                              checked={ephemeral}
                              onCheckedChange={(checked) => {
                                setEphemeral(checked === true);
                              }}
                            />
                            <Label htmlFor="ephemeral" className="font-medium">
                              {t("profiles.ephemeral")}
                            </Label>
                            <span className="px-1 py-0.5 text-[10px] leading-none rounded bg-muted text-muted-foreground font-medium">
                              {t("profiles.ephemeralAlpha")}
                            </span>
                          </div>
                          <p className="text-sm text-muted-foreground ml-6">
                            {t("profiles.ephemeralDescription")}
                          </p>
                        </div>

                        {selectedBrowser === "wayfern" ? (
                          // Wayfern Configuration
                          <div className="space-y-6">
                            {/* Wayfern Download Status */}
                            {isLoadingReleaseTypes && (
                              <div className="flex gap-3 items-center p-3 rounded-md border">
                                <div className="w-4 h-4 rounded-full border-2 animate-spin border-muted/40 border-t-primary" />
                                <p className="text-sm text-muted-foreground">
                                  Fetching available versions...
                                </p>
                              </div>
                            )}
                            {!isLoadingReleaseTypes && releaseTypesError && (
                              <div className="flex gap-3 items-center p-3 rounded-md border border-destructive/50 bg-destructive/10">
                                <p className="flex-1 text-sm text-destructive">
                                  {releaseTypesError}
                                </p>
                                <RippleButton
                                  onClick={() =>
                                    selectedBrowser &&
                                    loadReleaseTypes(selectedBrowser)
                                  }
                                  size="sm"
                                  variant="outline"
                                >
                                  Retry
                                </RippleButton>
                              </div>
                            )}
                            {!isLoadingReleaseTypes &&
                              !releaseTypesError &&
                              !getBestAvailableVersion("wayfern") && (
                                <div className="flex gap-3 items-center p-3 rounded-md border border-warning/50 bg-warning/10">
                                  <p className="text-sm text-warning">
                                    Wayfern is not available on your platform
                                    yet.
                                  </p>
                                </div>
                              )}
                            {!isLoadingReleaseTypes &&
                              !releaseTypesError &&
                              !isBrowserCurrentlyDownloading("wayfern") &&
                              !isBrowserVersionAvailable("wayfern") &&
                              getBestAvailableVersion("wayfern") && (
                                <div className="flex gap-3 items-center p-3 rounded-md border">
                                  <p className="text-sm text-muted-foreground">
                                    {(() => {
                                      const bestVersion =
                                        getBestAvailableVersion("wayfern");
                                      return `Wayfern version (${bestVersion?.version}) needs to be downloaded`;
                                    })()}
                                  </p>
                                  <LoadingButton
                                    onClick={() => {
                                      void handleDownload("wayfern");
                                    }}
                                    isLoading={isBrowserCurrentlyDownloading(
                                      "wayfern",
                                    )}
                                    size="sm"
                                    disabled={isBrowserCurrentlyDownloading(
                                      "wayfern",
                                    )}
                                  >
                                    {isBrowserCurrentlyDownloading("wayfern")
                                      ? "Downloading..."
                                      : "Download"}
                                  </LoadingButton>
                                </div>
                              )}
                            {!isLoadingReleaseTypes &&
                              !releaseTypesError &&
                              !isBrowserCurrentlyDownloading("wayfern") &&
                              isBrowserVersionAvailable("wayfern") && (
                                <div className="p-3 text-sm rounded-md border text-muted-foreground">
                                  {(() => {
                                    const bestVersion =
                                      getBestAvailableVersion("wayfern");
                                    return `✓ Wayfern version (${bestVersion?.version}) is available`;
                                  })()}
                                </div>
                              )}
                            {isBrowserCurrentlyDownloading("wayfern") && (
                              <div className="p-3 text-sm rounded-md border text-muted-foreground">
                                {(() => {
                                  const bestVersion =
                                    getBestAvailableVersion("wayfern");
                                  return `Downloading Wayfern version (${bestVersion?.version})...`;
                                })()}
                              </div>
                            )}

                            <WayfernConfigForm
                              config={wayfernConfig}
                              onConfigChange={updateWayfernConfig}
                              isCreating
                              crossOsUnlocked={crossOsUnlocked}
                              limitedMode={!crossOsUnlocked}
                              profileVersion={
                                getBestAvailableVersion("wayfern")?.version
                              }
                              profileBrowser="wayfern"
                            />
                          </div>
                        ) : selectedBrowser === "camoufox" ? (
                          // Camoufox Configuration
                          <div className="space-y-6">
                            {/* Camoufox Download Status */}
                            {isLoadingReleaseTypes && (
                              <div className="flex gap-3 items-center p-3 rounded-md border">
                                <div className="w-4 h-4 rounded-full border-2 animate-spin border-muted/40 border-t-primary" />
                                <p className="text-sm text-muted-foreground">
                                  Fetching available versions...
                                </p>
                              </div>
                            )}
                            {!isLoadingReleaseTypes && releaseTypesError && (
                              <div className="flex gap-3 items-center p-3 rounded-md border border-destructive/50 bg-destructive/10">
                                <p className="flex-1 text-sm text-destructive">
                                  {releaseTypesError}
                                </p>
                                <RippleButton
                                  onClick={() =>
                                    selectedBrowser &&
                                    loadReleaseTypes(selectedBrowser)
                                  }
                                  size="sm"
                                  variant="outline"
                                >
                                  Retry
                                </RippleButton>
                              </div>
                            )}
                            {!isLoadingReleaseTypes &&
                              !releaseTypesError &&
                              !getBestAvailableVersion("camoufox") && (
                                <div className="flex gap-3 items-center p-3 rounded-md border border-warning/50 bg-warning/10">
                                  <p className="text-sm text-warning">
                                    Camoufox is not available on your platform
                                    yet.
                                  </p>
                                </div>
                              )}
                            {!isLoadingReleaseTypes &&
                              !releaseTypesError &&
                              !isBrowserCurrentlyDownloading("camoufox") &&
                              !isBrowserVersionAvailable("camoufox") &&
                              getBestAvailableVersion("camoufox") && (
                                <div className="flex gap-3 items-center p-3 rounded-md border">
                                  <p className="text-sm text-muted-foreground">
                                    {(() => {
                                      const bestVersion =
                                        getBestAvailableVersion("camoufox");
                                      return `Camoufox version (${bestVersion?.version}) needs to be downloaded`;
                                    })()}
                                  </p>
                                  <LoadingButton
                                    onClick={() => {
                                      void handleDownload("camoufox");
                                    }}
                                    isLoading={isBrowserCurrentlyDownloading(
                                      "camoufox",
                                    )}
                                    size="sm"
                                    disabled={isBrowserCurrentlyDownloading(
                                      "camoufox",
                                    )}
                                  >
                                    {isBrowserCurrentlyDownloading("camoufox")
                                      ? "Downloading..."
                                      : "Download"}
                                  </LoadingButton>
                                </div>
                              )}
                            {!isLoadingReleaseTypes &&
                              !releaseTypesError &&
                              !isBrowserCurrentlyDownloading("camoufox") &&
                              isBrowserVersionAvailable("camoufox") && (
                                <div className="p-3 text-sm rounded-md border text-muted-foreground">
                                  {(() => {
                                    const bestVersion =
                                      getBestAvailableVersion("camoufox");
                                    return `✓ Camoufox version (${bestVersion?.version}) is available`;
                                  })()}
                                </div>
                              )}
                            {isBrowserCurrentlyDownloading("camoufox") && (
                              <div className="p-3 text-sm rounded-md border text-muted-foreground">
                                {(() => {
                                  const bestVersion =
                                    getBestAvailableVersion("camoufox");
                                  return `Downloading Camoufox version (${bestVersion?.version})...`;
                                })()}
                              </div>
                            )}

                            {crossOsUnlocked && (
                              <Alert className="border-warning/50 bg-warning/10">
                                <AlertDescription className="text-sm">
                                  {t("createProfile.camoufoxWarning")}
                                </AlertDescription>
                              </Alert>
                            )}

                            <SharedCamoufoxConfigForm
                              config={camoufoxConfig}
                              onConfigChange={updateCamoufoxConfig}
                              isCreating
                              browserType="camoufox"
                              crossOsUnlocked={crossOsUnlocked}
                              limitedMode={!crossOsUnlocked}
                              profileVersion={
                                getBestAvailableVersion("camoufox")?.version
                              }
                              profileBrowser="camoufox"
                            />
                          </div>
                        ) : (
                          // Regular Browser Configuration (should not happen in anti-detect tab)
                          <div className="space-y-4">
                            {selectedBrowser && (
                              <div className="space-y-3">
                                {isLoadingReleaseTypes && (
                                  <div className="flex gap-3 items-center">
                                    <div className="w-4 h-4 rounded-full border-2 animate-spin border-muted/40 border-t-primary" />
                                    <p className="text-sm text-muted-foreground">
                                      Fetching available versions...
                                    </p>
                                  </div>
                                )}
                                {!isLoadingReleaseTypes &&
                                  releaseTypesError && (
                                    <div className="flex gap-3 items-center p-3 rounded-md border border-destructive/50 bg-destructive/10">
                                      <p className="flex-1 text-sm text-destructive">
                                        {releaseTypesError}
                                      </p>
                                      <RippleButton
                                        onClick={() =>
                                          selectedBrowser &&
                                          loadReleaseTypes(selectedBrowser)
                                        }
                                        size="sm"
                                        variant="outline"
                                      >
                                        Retry
                                      </RippleButton>
                                    </div>
                                  )}
                                {!isLoadingReleaseTypes &&
                                  !releaseTypesError &&
                                  !isBrowserCurrentlyDownloading(
                                    selectedBrowser,
                                  ) &&
                                  !isBrowserVersionAvailable(selectedBrowser) &&
                                  getBestAvailableVersion(selectedBrowser) && (
                                    <div className="flex gap-3 items-center">
                                      <p className="text-sm text-muted-foreground">
                                        {(() => {
                                          const bestVersion =
                                            getBestAvailableVersion(
                                              selectedBrowser,
                                            );
                                          return `Latest version (${bestVersion?.version}) needs to be downloaded`;
                                        })()}
                                      </p>
                                      <LoadingButton
                                        onClick={() => {
                                          void handleDownload(selectedBrowser);
                                        }}
                                        isLoading={isBrowserCurrentlyDownloading(
                                          selectedBrowser,
                                        )}
                                        className="ml-auto"
                                        size="sm"
                                        disabled={isBrowserCurrentlyDownloading(
                                          selectedBrowser,
                                        )}
                                      >
                                        Download
                                      </LoadingButton>
                                    </div>
                                  )}
                                {!isLoadingReleaseTypes &&
                                  !releaseTypesError &&
                                  !isBrowserCurrentlyDownloading(
                                    selectedBrowser,
                                  ) &&
                                  isBrowserVersionAvailable(
                                    selectedBrowser,
                                  ) && (
                                    <div className="text-sm text-muted-foreground">
                                      {(() => {
                                        const bestVersion =
                                          getBestAvailableVersion(
                                            selectedBrowser,
                                          );
                                        return `✓ Latest version (${bestVersion?.version}) is available`;
                                      })()}
                                    </div>
                                  )}
                                {isBrowserCurrentlyDownloading(
                                  selectedBrowser,
                                ) && (
                                  <div className="text-sm text-muted-foreground">
                                    {(() => {
                                      const bestVersion =
                                        getBestAvailableVersion(
                                          selectedBrowser,
                                        );
                                      return `Downloading version (${bestVersion?.version})...`;
                                    })()}
                                  </div>
                                )}
                              </div>
                            )}
                          </div>
                        )}

                        {/* Proxy / VPN Selection - Always visible */}
                        <div className="space-y-3">
                          <div className="flex justify-between items-center">
                            <Label>Proxy / VPN</Label>
                            <RippleButton
                              size="sm"
                              variant="outline"
                              onClick={() => {
                                setShowProxyForm(true);
                              }}
                              className="px-2 h-7 text-xs"
                            >
                              <GoPlus className="mr-1 w-3 h-3" /> Add Proxy
                            </RippleButton>
                          </div>
                          {storedProxies.length > 0 || vpnConfigs.length > 0 ? (
                            <Popover
                              open={proxyPopoverOpen}
                              onOpenChange={setProxyPopoverOpen}
                            >
                              <PopoverTrigger asChild>
                                <Button
                                  variant="outline"
                                  role="combobox"
                                  aria-expanded={proxyPopoverOpen}
                                  className="w-full justify-between font-normal"
                                >
                                  {(() => {
                                    if (!selectedProxyId)
                                      return "No proxy / VPN";
                                    if (selectedProxyId.startsWith("vpn-")) {
                                      const vpn = vpnConfigs.find(
                                        (v) =>
                                          v.id === selectedProxyId.slice(4),
                                      );
                                      return vpn
                                        ? `WG — ${vpn.name}`
                                        : "No proxy / VPN";
                                    }
                                    const proxy = storedProxies.find(
                                      (p) => p.id === selectedProxyId,
                                    );
                                    return proxy?.name ?? "No proxy / VPN";
                                  })()}
                                  <LuChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
                                </Button>
                              </PopoverTrigger>
                              <PopoverContent
                                className="w-[240px] p-0"
                                sideOffset={8}
                              >
                                <Command>
                                  <CommandInput placeholder="Search proxies or VPNs..." />
                                  <CommandList>
                                    <CommandEmpty>
                                      No proxies or VPNs found.
                                    </CommandEmpty>
                                    <CommandGroup>
                                      <CommandItem
                                        value="__none__"
                                        onSelect={() => {
                                          setSelectedProxyId(undefined);
                                          setProxyPopoverOpen(false);
                                        }}
                                      >
                                        <LuCheck
                                          className={cn(
                                            "mr-2 h-4 w-4",
                                            !selectedProxyId
                                              ? "opacity-100"
                                              : "opacity-0",
                                          )}
                                        />
                                        None
                                      </CommandItem>
                                      {storedProxies.map((proxy) => (
                                        <CommandItem
                                          key={proxy.id}
                                          value={proxy.name}
                                          onSelect={() => {
                                            setSelectedProxyId(proxy.id);
                                            setProxyPopoverOpen(false);
                                          }}
                                        >
                                          <LuCheck
                                            className={cn(
                                              "mr-2 h-4 w-4",
                                              selectedProxyId === proxy.id
                                                ? "opacity-100"
                                                : "opacity-0",
                                            )}
                                          />
                                          {proxy.name}
                                        </CommandItem>
                                      ))}
                                    </CommandGroup>
                                    {vpnConfigs.length > 0 && (
                                      <CommandGroup heading="VPNs">
                                        {vpnConfigs.map((vpn) => (
                                          <CommandItem
                                            key={vpn.id}
                                            value={`vpn-${vpn.name}`}
                                            onSelect={() => {
                                              setSelectedProxyId(
                                                `vpn-${vpn.id}`,
                                              );
                                              setProxyPopoverOpen(false);
                                            }}
                                          >
                                            <LuCheck
                                              className={cn(
                                                "mr-2 h-4 w-4",
                                                selectedProxyId ===
                                                  `vpn-${vpn.id}`
                                                  ? "opacity-100"
                                                  : "opacity-0",
                                              )}
                                            />
                                            <Badge
                                              variant="outline"
                                              className="text-[10px] px-1 py-0 leading-tight mr-1"
                                            >
                                              WG
                                            </Badge>
                                            {vpn.name}
                                          </CommandItem>
                                        ))}
                                      </CommandGroup>
                                    )}
                                  </CommandList>
                                </Command>
                              </PopoverContent>
                            </Popover>
                          ) : (
                            <div className="flex gap-3 items-center p-3 text-sm rounded-md border text-muted-foreground">
                              No proxies or VPNs available. Add one to route
                              this profile's traffic.
                            </div>
                          )}
                        </div>

                        <div className="space-y-2">
                          <Label htmlFor="launch-hook-url">
                            {t("createProfile.launchHook.label")}
                          </Label>
                          <Input
                            id="launch-hook-url"
                            value={launchHook}
                            onChange={(e) => {
                              setLaunchHook(e.target.value);
                            }}
                            placeholder={t(
                              "createProfile.launchHook.placeholder",
                            )}
                            disabled={isCreating}
                          />
                        </div>

                        {/* DNS Blocklist */}
                        <div className="space-y-2">
                          <Label>{t("dnsBlocklist.title")}</Label>
                          <Select
                            value={dnsBlocklist || "none"}
                            onValueChange={(val) => {
                              setDnsBlocklist(val === "none" ? "" : val);
                            }}
                          >
                            <SelectTrigger>
                              <SelectValue
                                placeholder={t("dnsBlocklist.none")}
                              />
                            </SelectTrigger>
                            <SelectContent>
                              <SelectItem value="none">
                                {t("dnsBlocklist.none")}
                              </SelectItem>
                              <SelectItem value="light">
                                {t("dnsBlocklist.light")}
                              </SelectItem>
                              <SelectItem value="normal">
                                {t("dnsBlocklist.normal")}
                              </SelectItem>
                              <SelectItem value="pro">
                                {t("dnsBlocklist.pro")}
                              </SelectItem>
                              <SelectItem value="pro_plus">
                                {t("dnsBlocklist.proPlus")}
                              </SelectItem>
                              <SelectItem value="ultimate">
                                {t("dnsBlocklist.ultimate")}
                              </SelectItem>
                            </SelectContent>
                          </Select>
                        </div>

                        {/* Extension Group */}
                        {extensionGroups.length > 0 && (
                          <div className="space-y-2">
                            <Label>{t("extensions.extensionGroup")}</Label>
                            <Select
                              value={selectedExtensionGroupId ?? "none"}
                              onValueChange={(val) => {
                                setSelectedExtensionGroupId(
                                  val === "none" ? undefined : val,
                                );
                              }}
                            >
                              <SelectTrigger>
                                <SelectValue
                                  placeholder={t("profileInfo.values.none")}
                                />
                              </SelectTrigger>
                              <SelectContent>
                                <SelectItem value="none">
                                  {t("profileInfo.values.none")}
                                </SelectItem>
                                {extensionGroups.map((g) => (
                                  <SelectItem key={g.id} value={g.id}>
                                    {g.name} ({g.extension_ids.length})
                                  </SelectItem>
                                ))}
                              </SelectContent>
                            </Select>
                          </div>
                        )}
                      </div>
                    </TabsContent>

                    <TabsContent value="regular" className="mt-0">
                      {/* Regular Browser Configuration */}
                      <div className="space-y-6">
                        {/* Profile Name */}
                        <div className="space-y-2">
                          <Label htmlFor="profile-name">Profile Name</Label>
                          <Input
                            id="profile-name"
                            value={profileName}
                            onChange={(e) => {
                              setProfileName(e.target.value);
                            }}
                            onKeyDown={(e) => {
                              if (
                                e.key === "Enter" &&
                                !isCreateDisabled &&
                                !isCreating
                              ) {
                                void handleCreate();
                              }
                            }}
                            placeholder="Enter profile name"
                          />
                        </div>

                        {/* Regular Browser Configuration */}
                        <div className="space-y-4">
                          {selectedBrowser && (
                            <div className="space-y-3">
                              {isLoadingReleaseTypes && (
                                <div className="flex gap-3 items-center">
                                  <div className="w-4 h-4 rounded-full border-2 animate-spin border-muted/40 border-t-primary" />
                                  <p className="text-sm text-muted-foreground">
                                    Fetching available versions...
                                  </p>
                                </div>
                              )}
                              {!isLoadingReleaseTypes && releaseTypesError && (
                                <div className="flex gap-3 items-center p-3 rounded-md border border-destructive/50 bg-destructive/10">
                                  <p className="flex-1 text-sm text-destructive">
                                    {releaseTypesError}
                                  </p>
                                  <RippleButton
                                    onClick={() =>
                                      selectedBrowser &&
                                      loadReleaseTypes(selectedBrowser)
                                    }
                                    size="sm"
                                    variant="outline"
                                  >
                                    Retry
                                  </RippleButton>
                                </div>
                              )}
                              {!isLoadingReleaseTypes &&
                                !releaseTypesError &&
                                !isBrowserCurrentlyDownloading(
                                  selectedBrowser,
                                ) &&
                                !isBrowserVersionAvailable(selectedBrowser) &&
                                getBestAvailableVersion(selectedBrowser) && (
                                  <div className="flex gap-3 items-center">
                                    <p className="text-sm text-muted-foreground">
                                      {(() => {
                                        const bestVersion =
                                          getBestAvailableVersion(
                                            selectedBrowser,
                                          );
                                        return `Latest version (${bestVersion?.version}) needs to be downloaded`;
                                      })()}
                                    </p>
                                    <LoadingButton
                                      onClick={() => {
                                        void handleDownload(selectedBrowser);
                                      }}
                                      isLoading={isBrowserCurrentlyDownloading(
                                        selectedBrowser,
                                      )}
                                      className="ml-auto"
                                      size="sm"
                                      disabled={isBrowserCurrentlyDownloading(
                                        selectedBrowser,
                                      )}
                                    >
                                      Download
                                    </LoadingButton>
                                  </div>
                                )}
                              {!isLoadingReleaseTypes &&
                                !releaseTypesError &&
                                !isBrowserCurrentlyDownloading(
                                  selectedBrowser,
                                ) &&
                                isBrowserVersionAvailable(selectedBrowser) && (
                                  <div className="text-sm text-muted-foreground">
                                    {(() => {
                                      const bestVersion =
                                        getBestAvailableVersion(
                                          selectedBrowser,
                                        );
                                      return `✓ Latest version (${bestVersion?.version}) is available`;
                                    })()}
                                  </div>
                                )}
                              {isBrowserCurrentlyDownloading(
                                selectedBrowser,
                              ) && (
                                <div className="text-sm text-muted-foreground">
                                  {(() => {
                                    const bestVersion =
                                      getBestAvailableVersion(selectedBrowser);
                                    return `Downloading version (${bestVersion?.version})...`;
                                  })()}
                                </div>
                              )}
                            </div>
                          )}
                        </div>

                        {/* Proxy / VPN Selection - Always visible */}
                        <div className="space-y-3">
                          <div className="flex justify-between items-center">
                            <Label>Proxy / VPN</Label>
                            <RippleButton
                              size="sm"
                              variant="outline"
                              onClick={() => {
                                setShowProxyForm(true);
                              }}
                              className="px-2 h-7 text-xs"
                            >
                              <GoPlus className="mr-1 w-3 h-3" /> Add Proxy
                            </RippleButton>
                          </div>
                          {storedProxies.length > 0 || vpnConfigs.length > 0 ? (
                            <Popover
                              open={proxyPopoverOpen}
                              onOpenChange={setProxyPopoverOpen}
                            >
                              <PopoverTrigger asChild>
                                <Button
                                  variant="outline"
                                  role="combobox"
                                  aria-expanded={proxyPopoverOpen}
                                  className="w-full justify-between font-normal"
                                >
                                  {(() => {
                                    if (!selectedProxyId)
                                      return "No proxy / VPN";
                                    if (selectedProxyId.startsWith("vpn-")) {
                                      const vpn = vpnConfigs.find(
                                        (v) =>
                                          v.id === selectedProxyId.slice(4),
                                      );
                                      return vpn
                                        ? `WG — ${vpn.name}`
                                        : "No proxy / VPN";
                                    }
                                    const proxy = storedProxies.find(
                                      (p) => p.id === selectedProxyId,
                                    );
                                    return proxy?.name ?? "No proxy / VPN";
                                  })()}
                                  <LuChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
                                </Button>
                              </PopoverTrigger>
                              <PopoverContent
                                className="w-[240px] p-0"
                                sideOffset={8}
                              >
                                <Command>
                                  <CommandInput placeholder="Search proxies or VPNs..." />
                                  <CommandList>
                                    <CommandEmpty>
                                      No proxies or VPNs found.
                                    </CommandEmpty>
                                    <CommandGroup>
                                      <CommandItem
                                        value="__none__"
                                        onSelect={() => {
                                          setSelectedProxyId(undefined);
                                          setProxyPopoverOpen(false);
                                        }}
                                      >
                                        <LuCheck
                                          className={cn(
                                            "mr-2 h-4 w-4",
                                            !selectedProxyId
                                              ? "opacity-100"
                                              : "opacity-0",
                                          )}
                                        />
                                        None
                                      </CommandItem>
                                      {storedProxies.map((proxy) => (
                                        <CommandItem
                                          key={proxy.id}
                                          value={proxy.name}
                                          onSelect={() => {
                                            setSelectedProxyId(proxy.id);
                                            setProxyPopoverOpen(false);
                                          }}
                                        >
                                          <LuCheck
                                            className={cn(
                                              "mr-2 h-4 w-4",
                                              selectedProxyId === proxy.id
                                                ? "opacity-100"
                                                : "opacity-0",
                                            )}
                                          />
                                          {proxy.name}
                                        </CommandItem>
                                      ))}
                                    </CommandGroup>
                                    {vpnConfigs.length > 0 && (
                                      <CommandGroup heading="VPNs">
                                        {vpnConfigs.map((vpn) => (
                                          <CommandItem
                                            key={vpn.id}
                                            value={`vpn-${vpn.name}`}
                                            onSelect={() => {
                                              setSelectedProxyId(
                                                `vpn-${vpn.id}`,
                                              );
                                              setProxyPopoverOpen(false);
                                            }}
                                          >
                                            <LuCheck
                                              className={cn(
                                                "mr-2 h-4 w-4",
                                                selectedProxyId ===
                                                  `vpn-${vpn.id}`
                                                  ? "opacity-100"
                                                  : "opacity-0",
                                              )}
                                            />
                                            <Badge
                                              variant="outline"
                                              className="text-[10px] px-1 py-0 leading-tight mr-1"
                                            >
                                              WG
                                            </Badge>
                                            {vpn.name}
                                          </CommandItem>
                                        ))}
                                      </CommandGroup>
                                    )}
                                  </CommandList>
                                </Command>
                              </PopoverContent>
                            </Popover>
                          ) : (
                            <div className="flex gap-3 items-center p-3 text-sm rounded-md border text-muted-foreground">
                              No proxies or VPNs available. Add one to route
                              this profile's traffic.
                            </div>
                          )}
                        </div>

                        <div className="space-y-2">
                          <Label htmlFor="launch-hook-url-regular">
                            {t("createProfile.launchHook.label")}
                          </Label>
                          <Input
                            id="launch-hook-url-regular"
                            value={launchHook}
                            onChange={(e) => {
                              setLaunchHook(e.target.value);
                            }}
                            placeholder={t(
                              "createProfile.launchHook.placeholder",
                            )}
                            disabled={isCreating}
                          />
                        </div>
                      </div>
                    </TabsContent>
                  </>
                )}
              </div>
            </div>
          </ScrollArea>
        </Tabs>

        <DialogFooter className="flex-shrink-0 pt-4 border-t">
          {currentStep === "browser-config" ? (
            <>
              <RippleButton variant="outline" onClick={handleBack}>
                Back
              </RippleButton>
              <LoadingButton
                onClick={handleCreate}
                isLoading={isCreating}
                disabled={isCreateDisabled}
              >
                Create
              </LoadingButton>
            </>
          ) : (
            <RippleButton variant="outline" onClick={handleClose}>
              Cancel
            </RippleButton>
          )}
        </DialogFooter>
      </DialogContent>
      <ProxyFormDialog
        isOpen={showProxyForm}
        onClose={() => {
          setShowProxyForm(false);
        }}
      />
    </Dialog>
  );
}
