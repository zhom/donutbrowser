"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { GoPlus } from "react-icons/go";
import { LoadingButton } from "@/components/loading-button";
import { ProxyFormDialog } from "@/components/proxy-form-dialog";
import { SharedCamoufoxConfigForm } from "@/components/shared-camoufox-config-form";
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
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

import { useBrowserDownload } from "@/hooks/use-browser-download";
import { useProxyEvents } from "@/hooks/use-proxy-events";
import { getBrowserIcon } from "@/lib/browser-utils";
import type { BrowserReleaseTypes, CamoufoxConfig, CamoufoxOS } from "@/types";

const getCurrentOS = (): CamoufoxOS => {
  if (typeof navigator === "undefined") return "linux";
  const platform = navigator.platform.toLowerCase();
  if (platform.includes("win")) return "windows";
  if (platform.includes("mac")) return "macos";
  return "linux";
};

import { RippleButton } from "./ui/ripple";

type BrowserTypeString =
  | "mullvad-browser"
  | "firefox"
  | "firefox-developer"
  | "chromium"
  | "brave"
  | "zen"
  | "tor-browser"
  | "camoufox";

interface CreateProfileDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onCreateProfile: (profileData: {
    name: string;
    browserStr: BrowserTypeString;
    version: string;
    releaseType: string;
    proxyId?: string;
    camoufoxConfig?: CamoufoxConfig;
    groupId?: string;
  }) => Promise<void>;
  selectedGroupId?: string;
}

interface BrowserOption {
  value: BrowserTypeString;
  label: string;
}

const browserOptions: BrowserOption[] = [
  {
    value: "firefox",
    label: "Firefox",
  },
  {
    value: "firefox-developer",
    label: "Firefox Developer Edition",
  },
  {
    value: "chromium",
    label: "Chromium",
  },
  {
    value: "brave",
    label: "Brave",
  },
  {
    value: "zen",
    label: "Zen Browser",
  },
  {
    value: "mullvad-browser",
    label: "Mullvad Browser",
  },
  {
    value: "tor-browser",
    label: "Tor Browser",
  },
];

export function CreateProfileDialog({
  isOpen,
  onClose,
  onCreateProfile,
  selectedGroupId,
}: CreateProfileDialogProps) {
  const [profileName, setProfileName] = useState("");
  const [currentStep, setCurrentStep] = useState<
    "browser-selection" | "browser-config"
  >("browser-selection");
  const [activeTab, setActiveTab] = useState("anti-detect");

  // Browser selection states
  const [selectedBrowser, setSelectedBrowser] =
    useState<BrowserTypeString | null>(null);
  const [selectedProxyId, setSelectedProxyId] = useState<string>();

  // Camoufox anti-detect states
  const [camoufoxConfig, setCamoufoxConfig] = useState<CamoufoxConfig>(() => ({
    geoip: true, // Default to automatic geoip
    os: getCurrentOS(), // Default to current OS
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
  };

  const handleTabChange = (value: string) => {
    setActiveTab(value);
    setCurrentStep("browser-selection");
    setSelectedBrowser(null);
    setProfileName("");
    setSelectedProxyId(undefined);
  };

  const [supportedBrowsers, setSupportedBrowsers] = useState<string[]>([]);
  const { storedProxies } = useProxyEvents();
  const [showProxyForm, setShowProxyForm] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [releaseTypes, setReleaseTypes] = useState<BrowserReleaseTypes>();
  const loadingBrowserRef = useRef<string | null>(null);

  // Use the browser download hook
  const {
    isBrowserDownloading,
    downloadBrowser,
    loadDownloadedVersions,
    isVersionDownloaded,
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

      try {
        const rawReleaseTypes = await invoke<BrowserReleaseTypes>(
          "get_browser_release_types",
          { browserStr: browser },
        );

        await loadDownloadedVersions(browser);

        // Only update state if this browser is still the one we're loading
        if (loadingBrowserRef.current === browser) {
          // Filter to enforce stable-only creation, except Firefox Developer (nightly-only)
          if (browser === "camoufox") {
            const filtered: BrowserReleaseTypes = {};
            if (rawReleaseTypes.stable)
              filtered.stable = rawReleaseTypes.stable;
            setReleaseTypes(filtered);
          } else if (browser === "firefox-developer") {
            const filtered: BrowserReleaseTypes = {};
            if (rawReleaseTypes.nightly)
              filtered.nightly = rawReleaseTypes.nightly;
            setReleaseTypes(filtered);
          } else {
            const filtered: BrowserReleaseTypes = {};
            if (rawReleaseTypes.stable)
              filtered.stable = rawReleaseTypes.stable;
            setReleaseTypes(filtered);
          }
        }
      } catch (error) {
        console.error(`Failed to load release types for ${browser}:`, error);

        // Fallback: still load downloaded versions and derive release type from them if possible
        try {
          const downloaded = await loadDownloadedVersions(browser);
          if (loadingBrowserRef.current === browser && downloaded.length > 0) {
            const latest = downloaded[0];
            const fallback: BrowserReleaseTypes = {};
            if (browser === "firefox-developer") {
              fallback.nightly = latest;
            } else {
              fallback.stable = latest;
            }
            setReleaseTypes(fallback);
          }
        } catch (e) {
          console.error(
            `Failed to load downloaded versions for ${browser}:`,
            e,
          );
        }
      } finally {
        // Clear loading state only if we're still loading this browser
        if (loadingBrowserRef.current === browser) {
          loadingBrowserRef.current = null;
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
      // Check and download GeoIP database if needed for Camoufox
      if (selectedBrowser === "camoufox") {
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
    (browserType?: string) => {
      if (!releaseTypes) return null;

      // Firefox Developer Edition: nightly-only
      if (browserType === "firefox-developer" && releaseTypes.nightly) {
        return {
          version: releaseTypes.nightly,
          releaseType: "nightly" as const,
        };
      }
      // All others: stable-only
      if (releaseTypes.stable) {
        return { version: releaseTypes.stable, releaseType: "stable" as const };
      }
      return null;
    },
    [releaseTypes],
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
    try {
      if (activeTab === "anti-detect") {
        // Anti-detect browser - always use Camoufox with best available version
        const bestCamoufoxVersion = getBestAvailableVersion("camoufox");
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
          proxyId: selectedProxyId,
          camoufoxConfig: finalCamoufoxConfig,
          groupId: selectedGroupId !== "default" ? selectedGroupId : undefined,
        });
      } else {
        // Regular browser
        if (!selectedBrowser) {
          console.error("Missing required browser selection");
          return;
        }

        // Use the best available version (stable preferred, nightly as fallback)
        const bestVersion = getBestAvailableVersion(selectedBrowser);
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
    setReleaseTypes({});
    setCamoufoxConfig({
      geoip: true, // Reset to automatic geoip
      os: getCurrentOS(), // Reset to current OS
    });
    onClose();
  };

  const updateCamoufoxConfig = (key: keyof CamoufoxConfig, value: unknown) => {
    setCamoufoxConfig((prev) => ({ ...prev, [key]: value }));
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
    if (!isBrowserVersionAvailable(selectedBrowser)) return true;

    return false;
  }, [
    profileName,
    selectedBrowser,
    isBrowserCurrentlyDownloading,
    isBrowserVersionAvailable,
  ]);

  // Filter supported browsers for regular browsers (excluding mullvad and tor)
  const regularBrowsers = browserOptions.filter(
    (browser) =>
      supportedBrowsers.includes(browser.value) &&
      browser.value !== "mullvad-browser" &&
      browser.value !== "tor-browser",
  );

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="w-full max-h-[90vh] flex flex-col">
        <DialogHeader className="flex-shrink-0">
          <DialogTitle>
            {currentStep === "browser-selection"
              ? "Create New Profile"
              : "Configure Profile"}
          </DialogTitle>
        </DialogHeader>

        <Tabs
          value={activeTab}
          onValueChange={handleTabChange}
          className="flex flex-col flex-1 w-full min-h-0"
        >
          <TabsList
            className="grid flex-shrink-0 grid-cols-2 w-full"
            defaultValue="anti-detect"
          >
            <TabsTrigger value="anti-detect">Anti-Detect</TabsTrigger>
            <TabsTrigger value="regular">Regular</TabsTrigger>
          </TabsList>

          <ScrollArea className="overflow-y-auto flex-1">
            <div className="flex flex-col justify-center items-center w-full">
              <div className="py-4 space-y-6 w-full max-w-md">
                {currentStep === "browser-selection" ? (
                  <>
                    <TabsContent value="anti-detect" className="mt-0 space-y-6">
                      {/* Anti-Detect Browser Selection */}
                      <div className="space-y-6">
                        <div className="text-center">
                          <h3 className="text-lg font-medium">
                            Anti-Detect Browser
                          </h3>
                          <p className="mt-2 text-sm text-muted-foreground">
                            Choose Firefox for anti-detection capabilities
                          </p>
                        </div>

                        <Button
                          onClick={() => handleBrowserSelect("camoufox")}
                          className="flex gap-3 justify-start items-center p-4 w-full h-16 border-2 transition-colors hover:border-primary/50"
                          variant="outline"
                        >
                          <div className="flex justify-center items-center w-8 h-8">
                            {(() => {
                              const IconComponent = getBrowserIcon("firefox");
                              return IconComponent ? (
                                <IconComponent className="w-6 h-6" />
                              ) : null;
                            })()}
                          </div>
                          <div className="text-left">
                            <div className="font-medium">Firefox</div>
                            <div className="text-sm text-muted-foreground">
                              Anti-Detect Browser
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
                                onClick={() =>
                                  handleBrowserSelect(browser.value)
                                }
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
                            onChange={(e) => setProfileName(e.target.value)}
                            placeholder="Enter profile name"
                          />
                        </div>

                        {selectedBrowser === "camoufox" ? (
                          // Camoufox Configuration
                          <div className="space-y-6">
                            {/* Camoufox Download Status */}
                            {!isBrowserCurrentlyDownloading("camoufox") &&
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
                                    onClick={() => handleDownload("camoufox")}
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
                            {!isBrowserCurrentlyDownloading("camoufox") &&
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

                            <SharedCamoufoxConfigForm
                              config={camoufoxConfig}
                              onConfigChange={updateCamoufoxConfig}
                              isCreating
                            />
                          </div>
                        ) : (
                          // Regular Browser Configuration
                          <div className="space-y-4">
                            {selectedBrowser && (
                              <div className="space-y-3">
                                {!isBrowserCurrentlyDownloading(
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
                                        onClick={() =>
                                          handleDownload(selectedBrowser)
                                        }
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
                                {!isBrowserCurrentlyDownloading(
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

                        {/* Proxy Selection - Always visible */}
                        <div className="space-y-3">
                          <div className="flex justify-between items-center">
                            <Label>Proxy</Label>
                            <RippleButton
                              size="sm"
                              variant="outline"
                              onClick={() => setShowProxyForm(true)}
                              className="px-2 h-7 text-xs"
                            >
                              <GoPlus className="mr-1 w-3 h-3" /> Add Proxy
                            </RippleButton>
                          </div>
                          {storedProxies.length > 0 ? (
                            <Select
                              value={selectedProxyId || "none"}
                              onValueChange={(value) =>
                                setSelectedProxyId(
                                  value === "none" ? undefined : value,
                                )
                              }
                            >
                              <SelectTrigger>
                                <SelectValue placeholder="No proxy" />
                              </SelectTrigger>
                              <SelectContent>
                                <SelectItem value="none">No proxy</SelectItem>
                                {storedProxies.map((proxy) => (
                                  <SelectItem key={proxy.id} value={proxy.id}>
                                    {proxy.name}
                                  </SelectItem>
                                ))}
                              </SelectContent>
                            </Select>
                          ) : (
                            <div className="flex gap-3 items-center p-3 text-sm rounded-md border text-muted-foreground">
                              No proxies available. Add one to route this
                              profile's traffic.
                            </div>
                          )}
                        </div>
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
                            onChange={(e) => setProfileName(e.target.value)}
                            placeholder="Enter profile name"
                          />
                        </div>

                        {/* Regular Browser Configuration */}
                        <div className="space-y-4">
                          {selectedBrowser && (
                            <div className="space-y-3">
                              {!isBrowserCurrentlyDownloading(
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
                                      onClick={() =>
                                        handleDownload(selectedBrowser)
                                      }
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
                              {!isBrowserCurrentlyDownloading(
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

                        {/* Proxy Selection - Always visible */}
                        <div className="space-y-3">
                          <div className="flex justify-between items-center">
                            <Label>Proxy</Label>
                            <RippleButton
                              size="sm"
                              variant="outline"
                              onClick={() => setShowProxyForm(true)}
                              className="px-2 h-7 text-xs"
                            >
                              <GoPlus className="mr-1 w-3 h-3" /> Add Proxy
                            </RippleButton>
                          </div>
                          {storedProxies.length > 0 ? (
                            <Select
                              value={selectedProxyId || "none"}
                              onValueChange={(value) =>
                                setSelectedProxyId(
                                  value === "none" ? undefined : value,
                                )
                              }
                            >
                              <SelectTrigger>
                                <SelectValue placeholder="No proxy" />
                              </SelectTrigger>
                              <SelectContent>
                                <SelectItem value="none">No proxy</SelectItem>
                                {storedProxies.map((proxy) => (
                                  <SelectItem key={proxy.id} value={proxy.id}>
                                    {proxy.name}
                                  </SelectItem>
                                ))}
                              </SelectContent>
                            </Select>
                          ) : (
                            <div className="flex gap-3 items-center p-3 text-sm rounded-md border text-muted-foreground">
                              No proxies available. Add one to route this
                              profile's traffic.
                            </div>
                          )}
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
        onClose={() => setShowProxyForm(false)}
      />
    </Dialog>
  );
}
