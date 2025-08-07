"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { LoadingButton } from "@/components/loading-button";
import { SharedCamoufoxConfigForm } from "@/components/shared-camoufox-config-form";
import { Button } from "@/components/ui/button";
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
  const [activeTab, setActiveTab] = useState("regular");

  // Regular browser states
  const [selectedBrowser, setSelectedBrowser] = useState<BrowserTypeString>();
  const [selectedProxyId, setSelectedProxyId] = useState<string>();

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
          if (browser === "camoufox") {
            setCamoufoxReleaseTypes(releaseTypes);
          } else {
            setAvailableReleaseTypes(releaseTypes);
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
    }
  }, [isOpen, loadSupportedBrowsers, loadStoredProxies, loadReleaseTypes]);

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

  // Helper function to get the best available version and release type
  const getBestAvailableVersion = useCallback(
    (releaseTypes: BrowserReleaseTypes, browserType?: string) => {
      // For Firefox Developer Edition, prefer nightly over stable
      if (browserType === "firefox-developer" && releaseTypes.nightly) {
        return {
          version: releaseTypes.nightly,
          releaseType: "nightly" as const,
        };
      }

      if (releaseTypes.stable) {
        return { version: releaseTypes.stable, releaseType: "stable" as const };
      }
      if (releaseTypes.nightly) {
        return {
          version: releaseTypes.nightly,
          releaseType: "nightly" as const,
        };
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
    setActiveTab("regular");
    onClose();
  };

  const isCreateDisabled = () => {
    if (!profileName.trim()) return true;

    if (activeTab === "regular") {
      return (
        !selectedBrowser ||
        !getBestAvailableVersion(availableReleaseTypes, selectedBrowser)
      );
    } else {
      // For anti-detect, we need camoufox to be available
      return !getBestAvailableVersion(camoufoxReleaseTypes, "camoufox");
    }
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

  // No OS warning needed anymore

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-4xl max-h-[90vh] flex flex-col">
        <DialogHeader className="flex-shrink-0">
          <DialogTitle>Create New Profile</DialogTitle>
        </DialogHeader>

        <Tabs
          value={activeTab}
          onValueChange={setActiveTab}
          className="flex flex-col flex-1 w-full min-h-0"
        >
          <TabsList className="grid flex-shrink-0 grid-cols-2 w-full">
            <TabsTrigger value="regular">Regular Browsers</TabsTrigger>
            <TabsTrigger value="anti-detect">Anti-Detect</TabsTrigger>
          </TabsList>

          <ScrollArea className="flex-1 pr-6 h-[320px]">
            <div className="py-4 space-y-6">
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
                        .filter((browser) =>
                          supportedBrowsers.includes(browser.value),
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
                                return `${bestVersion?.releaseType === "stable" ? "Latest stable" : "Latest nightly"} version (${bestVersion?.version}) needs to be downloaded`;
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
                          <div className="text-sm text-green-600">
                            {(() => {
                              const bestVersion = getBestAvailableVersion(
                                availableReleaseTypes,
                                selectedBrowser,
                              );
                              return `✓ ${bestVersion?.releaseType === "stable" ? "Latest stable" : "Latest nightly"} version (${bestVersion?.version}) is available`;
                            })()}
                          </div>
                        )}
                      {isBrowserCurrentlyDownloading(selectedBrowser) && (
                        <div className="text-sm text-blue-600">
                          {(() => {
                            const bestVersion = getBestAvailableVersion(
                              availableReleaseTypes,
                              selectedBrowser,
                            );
                            return `Downloading ${bestVersion?.releaseType === "stable" ? "stable" : "nightly"} version (${bestVersion?.version})...`;
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
                      <div className="flex gap-3 items-center p-3 bg-amber-50 rounded-md border border-amber-200">
                        <p className="text-sm text-amber-800">
                          {(() => {
                            const bestVersion = getBestAvailableVersion(
                              camoufoxReleaseTypes,
                              "camoufox",
                            );
                            return `Camoufox ${bestVersion?.releaseType} version (${bestVersion?.version}) needs to be downloaded`;
                          })()}
                        </p>
                        <LoadingButton
                          onClick={() => handleDownload("camoufox")}
                          isLoading={isBrowserCurrentlyDownloading("camoufox")}
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
                      <div className="p-3 text-sm text-green-600 bg-green-50 rounded-md border border-green-200">
                        {(() => {
                          const bestVersion = getBestAvailableVersion(
                            camoufoxReleaseTypes,
                            "camoufox",
                          );
                          return `✓ Camoufox ${bestVersion?.releaseType} version (${bestVersion?.version}) is available`;
                        })()}
                      </div>
                    )}
                  {isBrowserCurrentlyDownloading("camoufox") && (
                    <div className="p-3 text-sm text-blue-600 bg-blue-50 rounded-md border border-blue-200">
                      {(() => {
                        const bestVersion = getBestAvailableVersion(
                          camoufoxReleaseTypes,
                          "camoufox",
                        );
                        return `Downloading Camoufox ${bestVersion?.releaseType} version (${bestVersion?.version})...`;
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

              {/* Proxy Selection - Common to both tabs - Compact without card */}
              {storedProxies.length > 0 && (
                <div className="space-y-3">
                  <Label>Proxy</Label>
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
                          {proxy.name}{" "}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              )}
            </div>
          </ScrollArea>

          <DialogFooter className="flex-shrink-0 pt-4 border-t">
            <Button variant="outline" onClick={handleClose}>
              Cancel
            </Button>
            <LoadingButton
              onClick={handleCreate}
              isLoading={isCreating}
              disabled={isCreateDisabled()}
            >
              Create Profile
            </LoadingButton>
          </DialogFooter>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}
