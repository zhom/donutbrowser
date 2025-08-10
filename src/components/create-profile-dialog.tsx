"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { GoPlus } from "react-icons/go";
import { LoadingButton } from "@/components/loading-button";
import { ProxyFormDialog } from "@/components/proxy-form-dialog";
import { SharedCamoufoxConfigForm } from "@/components/shared-camoufox-config-form";
import { Combobox } from "@/components/ui/combobox";
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
import { getBrowserIcon } from "@/lib/browser-utils";
import type { BrowserReleaseTypes, CamoufoxConfig, StoredProxy } from "@/types";
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
  description: string;
}

const browserOptions: BrowserOption[] = [
  {
    value: "firefox",
    label: "Firefox",
    description: "Mozilla's main web browser",
  },
  {
    value: "firefox-developer",
    label: "Firefox Developer Edition",
    description: "Browser for developers with cutting-edge features",
  },
  {
    value: "chromium",
    label: "Chromium",
    description: "Open-source version of Chrome",
  },
  {
    value: "brave",
    label: "Brave",
    description: "Privacy-focused browser with ad blocking",
  },
  {
    value: "zen",
    label: "Zen Browser",
    description: "Beautiful, customizable Firefox-based browser",
  },
  {
    value: "mullvad-browser",
    label: "Mullvad Browser",
    description: "TOR Browser fork by Mullvad VPN",
  },
  {
    value: "tor-browser",
    label: "Tor Browser",
    description: "Browse anonymously through the Tor network",
  },
];

export function CreateProfileDialog({
  isOpen,
  onClose,
  onCreateProfile,
  selectedGroupId,
}: CreateProfileDialogProps) {
  const [profileName, setProfileName] = useState("");
  const [activeTab, setActiveTab] = useState("anti-detect");

  // Regular browser states
  const [selectedBrowser, setSelectedBrowser] = useState<BrowserTypeString>();
  const [selectedProxyId, setSelectedProxyId] = useState<string>();

  const handleTabChange = (value: string) => {
    if (value === "regular") {
      setSelectedBrowser(undefined);
    } else if (value === "anti-detect") {
      setSelectedBrowser("camoufox");
    }

    setActiveTab(value);
  };

  // Camoufox anti-detect states
  const [camoufoxConfig, setCamoufoxConfig] = useState<CamoufoxConfig>({
    geoip: true, // Default to automatic geoip
  });

  // Common states
  const [availableReleaseTypes, setAvailableReleaseTypes] =
    useState<BrowserReleaseTypes>({});
  const [camoufoxReleaseTypes, setCamoufoxReleaseTypes] =
    useState<BrowserReleaseTypes>({});
  const [supportedBrowsers, setSupportedBrowsers] = useState<string[]>([]);
  const [storedProxies, setStoredProxies] = useState<StoredProxy[]>([]);
  const [showProxyForm, setShowProxyForm] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
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

  const loadStoredProxies = useCallback(async () => {
    try {
      const proxies = await invoke<StoredProxy[]>("get_stored_proxies");
      setStoredProxies(proxies);
    } catch (error) {
      console.error("Failed to load stored proxies:", error);
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
        const releaseTypes = await invoke<BrowserReleaseTypes>(
          "get_browser_release_types",
          { browserStr: browser },
        );

        // Only update state if this browser is still the one we're loading
        if (loadingBrowserRef.current === browser) {
          // Filter to enforce stable-only creation, except Firefox Developer (nightly-only)
          if (browser === "camoufox") {
            const filtered: BrowserReleaseTypes = {};
            if (releaseTypes.stable) filtered.stable = releaseTypes.stable;
            setCamoufoxReleaseTypes(filtered);
          } else if (browser === "firefox-developer") {
            const filtered: BrowserReleaseTypes = {};
            if (releaseTypes.nightly) filtered.nightly = releaseTypes.nightly;
            setAvailableReleaseTypes(filtered);
          } else {
            const filtered: BrowserReleaseTypes = {};
            if (releaseTypes.stable) filtered.stable = releaseTypes.stable;
            setAvailableReleaseTypes(filtered);
          }

          // Load downloaded versions for this browser
          await loadDownloadedVersions(browser);
        }
      } catch (error) {
        console.error(`Failed to load release types for ${browser}:`, error);
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
      void loadStoredProxies();
      // Load camoufox release types when dialog opens
      void loadReleaseTypes("camoufox");
      // Check and download GeoIP database if needed for Camoufox
      void checkAndDownloadGeoIPDatabase();
    }
  }, [
    isOpen,
    loadSupportedBrowsers,
    loadStoredProxies,
    loadReleaseTypes,
    checkAndDownloadGeoIPDatabase,
  ]);

  // Load release types when browser selection changes
  useEffect(() => {
    if (selectedBrowser) {
      // Cancel any previous loading
      loadingBrowserRef.current = null;
      // Clear previous release types immediately to prevent showing stale data
      setAvailableReleaseTypes({});
      void loadReleaseTypes(selectedBrowser);
    }
  }, [selectedBrowser, loadReleaseTypes]);

  // Helper function to get the best available version respecting rules
  const getBestAvailableVersion = useCallback(
    (releaseTypes: BrowserReleaseTypes, browserType?: string) => {
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
    [],
  );

  const handleDownload = async (browserStr: string) => {
    const releaseTypes =
      browserStr === "camoufox" ? camoufoxReleaseTypes : availableReleaseTypes;
    const bestVersion = getBestAvailableVersion(releaseTypes, browserStr);

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
      if (activeTab === "regular") {
        if (!selectedBrowser) {
          console.error("Missing required browser selection");
          return;
        }

        // Use the best available version (stable preferred, nightly as fallback)
        const bestVersion = getBestAvailableVersion(
          availableReleaseTypes,
          selectedBrowser,
        );
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
      } else {
        // Anti-detect tab - always use Camoufox with best available version
        const bestCamoufoxVersion = getBestAvailableVersion(
          camoufoxReleaseTypes,
          "camoufox",
        );
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
    setSelectedBrowser(undefined);
    setSelectedProxyId(undefined);
    setAvailableReleaseTypes({});
    setCamoufoxReleaseTypes({});
    setCamoufoxConfig({
      geoip: true, // Reset to automatic geoip
    });
    setActiveTab("anti-detect");
    onClose();
  };

  const isCreateDisabled = () => {
    if (!profileName.trim()) return true;

    if (!selectedBrowser) return true;

    if (isBrowserCurrentlyDownloading(selectedBrowser)) return true;

    if (!isBrowserVersionAvailable(selectedBrowser)) return true;

    if (selectedBrowser === "camoufox") {
      return !getBestAvailableVersion(camoufoxReleaseTypes, selectedBrowser);
    }

    return !getBestAvailableVersion(availableReleaseTypes, selectedBrowser);
  };

  const updateCamoufoxConfig = (key: keyof CamoufoxConfig, value: unknown) => {
    setCamoufoxConfig((prev) => ({ ...prev, [key]: value }));
  };

  // Check if browser version is downloaded and available
  const isBrowserVersionAvailable = (browserStr: string) => {
    const releaseTypes =
      browserStr === "camoufox" ? camoufoxReleaseTypes : availableReleaseTypes;
    const bestVersion = getBestAvailableVersion(releaseTypes, browserStr);
    return bestVersion && isVersionDownloaded(bestVersion.version);
  };

  // Check if browser is currently downloading
  const isBrowserCurrentlyDownloading = (browserStr: string) => {
    return isBrowserDownloading(browserStr);
  };

  console.log(selectedBrowser);

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="w-full max-h-[90vh] flex flex-col">
        <DialogHeader className="flex-shrink-0">
          <DialogTitle>Create New Profile</DialogTitle>
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

          <ScrollArea className="flex-1 h-[330px] overflow-y-hidden">
            <div className="flex flex-col justify-center items-center w-full">
              <div className="py-4 space-y-6 w-full max-w-md">
                {/* Profile Name - Common to both tabs */}
                <div className="space-y-2">
                  <Label htmlFor="profile-name">Profile Name</Label>
                  <Input
                    id="profile-name"
                    value={profileName}
                    onChange={(e) => setProfileName(e.target.value)}
                    placeholder="Enter profile name"
                  />
                </div>

                <TabsContent value="regular" className="mt-0 space-y-6">
                  <div className="space-y-4">
                    <div className="space-y-2">
                      <Label>Browser</Label>
                      <Combobox
                        options={browserOptions
                          .filter(
                            (browser) =>
                              supportedBrowsers.includes(browser.value) &&
                              browser.value !== "mullvad-browser" &&
                              browser.value !== "tor-browser",
                          )
                          .map((browser) => {
                            const IconComponent = getBrowserIcon(browser.value);
                            return {
                              value: browser.value,
                              label: browser.label,
                              icon: IconComponent,
                            };
                          })}
                        value={selectedBrowser || ""}
                        onValueChange={(value) =>
                          setSelectedBrowser(value as BrowserTypeString)
                        }
                        placeholder="Select a browser..."
                        searchPlaceholder="Search browsers..."
                      />
                    </div>

                    {selectedBrowser && (
                      <div className="space-y-3">
                        {!isBrowserCurrentlyDownloading(selectedBrowser) &&
                          !isBrowserVersionAvailable(selectedBrowser) &&
                          getBestAvailableVersion(
                            availableReleaseTypes,
                            selectedBrowser,
                          ) && (
                            <div className="flex gap-3 items-center">
                              <p className="text-sm text-muted-foreground">
                                {(() => {
                                  const bestVersion = getBestAvailableVersion(
                                    availableReleaseTypes,
                                    selectedBrowser,
                                  );
                                  return `Latest version (${bestVersion?.version}) needs to be downloaded`;
                                })()}
                              </p>
                              <LoadingButton
                                onClick={() => handleDownload(selectedBrowser)}
                                isLoading={isBrowserCurrentlyDownloading(
                                  selectedBrowser,
                                )}
                                size="sm"
                                disabled={isBrowserCurrentlyDownloading(
                                  selectedBrowser,
                                )}
                              >
                                Download
                              </LoadingButton>
                            </div>
                          )}
                        {!isBrowserCurrentlyDownloading(selectedBrowser) &&
                          isBrowserVersionAvailable(selectedBrowser) && (
                            <div className="text-sm text-muted-foreground">
                              {(() => {
                                const bestVersion = getBestAvailableVersion(
                                  availableReleaseTypes,
                                  selectedBrowser,
                                );
                                return `✓ Latest version (${bestVersion?.version}) is available`;
                              })()}
                            </div>
                          )}
                        {isBrowserCurrentlyDownloading(selectedBrowser) && (
                          <div className="text-sm text-muted-foreground">
                            {(() => {
                              const bestVersion = getBestAvailableVersion(
                                availableReleaseTypes,
                                selectedBrowser,
                              );
                              return `Downloading version (${bestVersion?.version})...`;
                            })()}
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                </TabsContent>

                <TabsContent value="anti-detect" className="mt-0 space-y-6">
                  <div className="space-y-6">
                    {/* Camoufox Download Status */}
                    {!isBrowserCurrentlyDownloading("camoufox") &&
                      !isBrowserVersionAvailable("camoufox") &&
                      getBestAvailableVersion(
                        camoufoxReleaseTypes,
                        "camoufox",
                      ) && (
                        <div className="flex gap-3 items-center p-3 rounded-md border">
                          <p className="text-sm text-muted-foreground">
                            {(() => {
                              const bestVersion = getBestAvailableVersion(
                                camoufoxReleaseTypes,
                                "camoufox",
                              );
                              return `Camoufox version (${bestVersion?.version}) needs to be downloaded`;
                            })()}
                          </p>
                          <LoadingButton
                            onClick={() => handleDownload("camoufox")}
                            isLoading={isBrowserCurrentlyDownloading(
                              "camoufox",
                            )}
                            size="sm"
                            disabled={isBrowserCurrentlyDownloading("camoufox")}
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
                            const bestVersion = getBestAvailableVersion(
                              camoufoxReleaseTypes,
                              "camoufox",
                            );
                            return `✓ Camoufox version (${bestVersion?.version}) is available`;
                          })()}
                        </div>
                      )}
                    {isBrowserCurrentlyDownloading("camoufox") && (
                      <div className="p-3 text-sm rounded-md border text-muted-foreground">
                        {(() => {
                          const bestVersion = getBestAvailableVersion(
                            camoufoxReleaseTypes,
                            "camoufox",
                          );
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
                </TabsContent>

                {/* Proxy Selection - Common to both tabs - Always visible */}
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
                        setSelectedProxyId(value === "none" ? undefined : value)
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
                      No proxies available. Add one to route this profile's
                      traffic.
                    </div>
                  )}
                </div>
              </div>
            </div>
          </ScrollArea>

          <DialogFooter className="flex-shrink-0 pt-4 border-t">
            <RippleButton variant="outline" onClick={handleClose}>
              Cancel
            </RippleButton>
            <LoadingButton
              onClick={handleCreate}
              isLoading={isCreating}
              disabled={isCreateDisabled()}
            >
              Create
            </LoadingButton>
          </DialogFooter>
        </Tabs>
      </DialogContent>
      <ProxyFormDialog
        isOpen={showProxyForm}
        onClose={() => setShowProxyForm(false)}
        onSave={(proxy) => {
          setStoredProxies((prev) => [...prev, proxy]);
          setSelectedProxyId(proxy.id);
        }}
      />
    </Dialog>
  );
}
