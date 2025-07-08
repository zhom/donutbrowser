"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
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
import { getBrowserIcon, getCurrentOS } from "@/lib/browser-utils";
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
  }) => Promise<void>;
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
    description: "Privacy browser by Mullvad VPN",
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
}: CreateProfileDialogProps) {
  const [profileName, setProfileName] = useState("");
  const [activeTab, setActiveTab] = useState("regular");

  // Regular browser states
  const [selectedBrowser, setSelectedBrowser] = useState<BrowserTypeString>();
  const [selectedProxyId, setSelectedProxyId] = useState<string>();

  // Camoufox anti-detect states
  const [camoufoxConfig, setCamoufoxConfig] = useState<CamoufoxConfig>({
    enable_cache: true, // Cache enabled by default
    os: [getCurrentOS()], // Default to current OS
  });

  // Common states
  const [availableReleaseTypes, setAvailableReleaseTypes] =
    useState<BrowserReleaseTypes>({});
  const [camoufoxReleaseTypes, setCamoufoxReleaseTypes] =
    useState<BrowserReleaseTypes>({});
  const [supportedBrowsers, setSupportedBrowsers] = useState<string[]>([]);
  const [storedProxies, setStoredProxies] = useState<StoredProxy[]>([]);
  const [isCreating, setIsCreating] = useState(false);

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
      try {
        const releaseTypes = await invoke<BrowserReleaseTypes>(
          "get_browser_release_types",
          { browserStr: browser },
        );

        if (browser === "camoufox") {
          setCamoufoxReleaseTypes(releaseTypes);
        } else {
          setAvailableReleaseTypes(releaseTypes);
        }

        // Load downloaded versions for this browser
        await loadDownloadedVersions(browser);
      } catch (error) {
        console.error(`Failed to load release types for ${browser}:`, error);
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
      void loadReleaseTypes(selectedBrowser);
    }
  }, [selectedBrowser, loadReleaseTypes]);

  const handleDownload = async (browserStr: string) => {
    const releaseTypes =
      browserStr === "camoufox" ? camoufoxReleaseTypes : availableReleaseTypes;
    const latestStableVersion = releaseTypes.stable;

    if (!latestStableVersion) {
      console.error("No stable version available for download");
      return;
    }

    try {
      await downloadBrowser(browserStr, latestStableVersion);
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

        // Use the latest stable version by default
        const latestStableVersion = availableReleaseTypes.stable;
        if (!latestStableVersion) {
          console.error("No stable version available");
          return;
        }

        await onCreateProfile({
          name: profileName.trim(),
          browserStr: selectedBrowser,
          version: latestStableVersion,
          releaseType: "stable",
          proxyId: selectedProxyId,
        });
      } else {
        // Anti-detect tab - always use Camoufox with latest version
        const latestCamoufoxVersion = camoufoxReleaseTypes.stable;
        if (!latestCamoufoxVersion) {
          console.error("No Camoufox version available");
          return;
        }

        await onCreateProfile({
          name: profileName.trim(),
          browserStr: "camoufox" as BrowserTypeString,
          version: latestCamoufoxVersion,
          releaseType: "stable",
          proxyId: selectedProxyId,
          camoufoxConfig,
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
    // Reset all states
    setProfileName("");
    setSelectedBrowser(undefined);
    setSelectedProxyId(undefined);
    setCamoufoxConfig({
      enable_cache: true,
      os: [getCurrentOS()], // Reset to current OS
    });
    setActiveTab("regular");
    onClose();
  };

  const isCreateDisabled = () => {
    if (!profileName.trim()) return true;

    if (activeTab === "regular") {
      return !selectedBrowser || !availableReleaseTypes.stable;
    } else {
      // For anti-detect, we need camoufox to be available
      return !camoufoxReleaseTypes.stable;
    }
  };

  const updateCamoufoxConfig = (key: keyof CamoufoxConfig, value: unknown) => {
    setCamoufoxConfig((prev) => ({ ...prev, [key]: value }));
  };

  // Check if browser version is downloaded and available
  const isBrowserVersionAvailable = (browserStr: string) => {
    const releaseTypes =
      browserStr === "camoufox" ? camoufoxReleaseTypes : availableReleaseTypes;
    const latestStableVersion = releaseTypes.stable;
    return latestStableVersion && isVersionDownloaded(latestStableVersion);
  };

  // Get the selected OS for warning
  const selectedOS = camoufoxConfig.os?.[0];
  const currentOS = getCurrentOS();
  const _showOSWarning =
    selectedOS && selectedOS !== currentOS && currentOS !== "unknown";

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
                      {!isBrowserVersionAvailable(selectedBrowser) &&
                        availableReleaseTypes.stable && (
                          <div className="flex gap-3 items-center">
                            <p className="text-sm text-muted-foreground">
                              Latest stable version (
                              {availableReleaseTypes.stable}) needs to be
                              downloaded
                            </p>
                            <LoadingButton
                              onClick={() => handleDownload(selectedBrowser)}
                              isLoading={isBrowserDownloading(selectedBrowser)}
                              size="sm"
                              disabled={isBrowserDownloading(selectedBrowser)}
                            >
                              Download
                            </LoadingButton>
                          </div>
                        )}
                      {isBrowserVersionAvailable(selectedBrowser) && (
                        <div className="text-sm text-green-600">
                          ✓ Latest stable version (
                          {availableReleaseTypes.stable}) is available
                        </div>
                      )}
                    </div>
                  )}
                </div>
              </TabsContent>

              <TabsContent value="anti-detect" className="mt-0 space-y-6">
                {/* Anti-Detect Description */}
                <div className="p-3 text-center bg-blue-50 rounded-md border border-blue-200 dark:bg-blue-950 dark:border-blue-800">
                  <p className="text-sm text-blue-800 dark:text-blue-200">
                    Powered by Camoufox
                  </p>
                </div>

                <div className="space-y-6">
                  {/* Camoufox Download Status */}
                  {!isBrowserVersionAvailable("camoufox") &&
                    camoufoxReleaseTypes.stable && (
                      <div className="flex gap-3 items-center p-3 bg-amber-50 rounded-md border border-amber-200">
                        <p className="text-sm text-amber-800">
                          Camoufox version ({camoufoxReleaseTypes.stable}) needs
                          to be downloaded
                        </p>
                        <LoadingButton
                          onClick={() => handleDownload("camoufox")}
                          isLoading={isBrowserDownloading("camoufox")}
                          size="sm"
                          disabled={isBrowserDownloading("camoufox")}
                        >
                          Download
                        </LoadingButton>
                      </div>
                    )}
                  {isBrowserVersionAvailable("camoufox") && (
                    <div className="p-3 text-sm text-green-600 bg-green-50 rounded-md border border-green-200">
                      ✓ Camoufox version ({camoufoxReleaseTypes.stable}) is
                      available
                    </div>
                  )}

                  <SharedCamoufoxConfigForm
                    config={camoufoxConfig}
                    onConfigChange={updateCamoufoxConfig}
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
                          {proxy.name} ({proxy.proxy_settings.proxy_type}://
                          {proxy.proxy_settings.host}:
                          {proxy.proxy_settings.port})
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
