"use client";

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { FaFolder } from "react-icons/fa";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import { SharedCamoufoxConfigForm } from "@/components/shared-camoufox-config-form";
import { Alert, AlertDescription } from "@/components/ui/alert";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { WayfernConfigForm } from "@/components/wayfern-config-form";
import { useBrowserSupport } from "@/hooks/use-browser-support";
import { useProxyEvents } from "@/hooks/use-proxy-events";
import { getBrowserDisplayName, getBrowserIcon } from "@/lib/browser-utils";
import type { CamoufoxConfig, DetectedProfile, WayfernConfig } from "@/types";
import { RippleButton } from "./ui/ripple";

const getMappedBrowser = (browser: string): "camoufox" | "wayfern" => {
  if (["firefox", "firefox-developer", "zen"].includes(browser))
    return "camoufox";
  return "wayfern";
};

interface ImportProfileDialogProps {
  isOpen: boolean;
  onClose: () => void;
  crossOsUnlocked?: boolean;
}

export function ImportProfileDialog({
  isOpen,
  onClose,
  crossOsUnlocked,
}: ImportProfileDialogProps) {
  const { t } = useTranslation();
  const [detectedProfiles, setDetectedProfiles] = useState<DetectedProfile[]>(
    [],
  );
  const [isLoading, setIsLoading] = useState(false);
  const [isImporting, setIsImporting] = useState(false);
  const [importMode, setImportMode] = useState<"auto-detect" | "manual">(
    "auto-detect",
  );
  const [currentStep, setCurrentStep] = useState<"select" | "configure">(
    "select",
  );
  const [camoufoxConfig, setCamoufoxConfig] = useState<CamoufoxConfig>({});
  const [wayfernConfig, setWayfernConfig] = useState<WayfernConfig>({});
  const [selectedProxyId, setSelectedProxyId] = useState<string | undefined>();

  // Auto-detect state
  const [selectedDetectedProfile, setSelectedDetectedProfile] = useState<
    string | null
  >(null);
  const [autoDetectProfileName, setAutoDetectProfileName] = useState("");

  // Manual import state
  const [manualBrowserType, setManualBrowserType] = useState<string | null>(
    null,
  );
  const [manualProfilePath, setManualProfilePath] = useState("");
  const [manualProfileName, setManualProfileName] = useState("");

  const { supportedBrowsers, isLoading: isLoadingSupport } =
    useBrowserSupport();
  const { storedProxies } = useProxyEvents();

  const importableBrowsers = supportedBrowsers;

  const loadDetectedProfiles = useCallback(async () => {
    setIsLoading(true);
    try {
      const profiles = await invoke<DetectedProfile[]>(
        "detect_existing_profiles",
      );
      setDetectedProfiles(profiles);

      if (profiles.length === 0) {
        setImportMode("manual");
      } else {
        setSelectedDetectedProfile(profiles[0].path);

        const profile = profiles[0];
        const browserName = getBrowserDisplayName(profile.browser);
        const defaultName = `Imported ${browserName} Profile`;
        setAutoDetectProfileName(defaultName);
      }
    } catch (error) {
      console.error("Failed to detect existing profiles:", error);
      toast.error(t("importProfile.detectFailed"));
    } finally {
      setIsLoading(false);
    }
  }, [t]);

  const selectedProfile = detectedProfiles.find(
    (p) => p.path === selectedDetectedProfile,
  );

  const handleBrowseFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: t("importProfile.selectFolderTitle"),
      });

      if (selected && typeof selected === "string") {
        setManualProfilePath(selected);
      }
    } catch (error) {
      console.error("Failed to open folder dialog:", error);
      toast.error(t("importProfile.folderDialogFailed"));
    }
  };

  const handleImport = useCallback(async () => {
    let sourcePath: string;
    let browserType: string;
    let newProfileName: string;

    if (importMode === "auto-detect") {
      if (!selectedDetectedProfile || !autoDetectProfileName.trim()) {
        toast.error(t("importProfile.selectAndName"));
        return;
      }
      const profile = detectedProfiles.find(
        (p) => p.path === selectedDetectedProfile,
      );
      if (!profile) {
        toast.error(t("importProfile.profileNotFound"));
        return;
      }
      sourcePath = profile.path;
      browserType = profile.browser;
      newProfileName = autoDetectProfileName.trim();
    } else {
      if (
        !manualBrowserType ||
        !manualProfilePath.trim() ||
        !manualProfileName.trim()
      ) {
        toast.error(t("importProfile.fillFields"));
        return;
      }
      sourcePath = manualProfilePath.trim();
      browserType = manualBrowserType;
      newProfileName = manualProfileName.trim();
    }

    const mappedBrowser =
      importMode === "auto-detect" && selectedProfile
        ? (selectedProfile.mapped_browser as "camoufox" | "wayfern")
        : getMappedBrowser(browserType);

    setIsImporting(true);
    try {
      await invoke("import_browser_profile", {
        sourcePath,
        browserType,
        newProfileName,
        proxyId: selectedProxyId ?? null,
        camoufoxConfig: mappedBrowser === "camoufox" ? camoufoxConfig : null,
        wayfernConfig: mappedBrowser === "wayfern" ? wayfernConfig : null,
      });

      toast.success(
        t("importProfile.importedSuccess", { name: newProfileName }),
      );
      onClose();
    } catch (error) {
      console.error("Failed to import profile:", error);
      const errorMessage =
        error instanceof Error ? error.message : String(error);

      if (errorMessage.includes("No downloaded versions found")) {
        const browserDisplayName = getBrowserDisplayName(browserType);
        toast.error(
          t("importProfile.notInstalled", { browser: browserDisplayName }),
          {
            duration: 8000,
          },
        );
      } else {
        toast.error(t("importProfile.importFailed", { error: errorMessage }));
      }
    } finally {
      setIsImporting(false);
    }
  }, [
    importMode,
    selectedDetectedProfile,
    autoDetectProfileName,
    detectedProfiles,
    manualBrowserType,
    manualProfilePath,
    manualProfileName,
    selectedProxyId,
    camoufoxConfig,
    wayfernConfig,
    onClose,
    selectedProfile,
    t,
  ]);

  const handleClose = () => {
    setCurrentStep("select");
    setCamoufoxConfig({});
    setWayfernConfig({});
    setSelectedProxyId(undefined);
    setSelectedDetectedProfile(null);
    setAutoDetectProfileName("");
    setManualBrowserType(null);
    setManualProfilePath("");
    setManualProfileName("");
    if (detectedProfiles.length > 0) {
      setImportMode("auto-detect");
    } else {
      setImportMode("manual");
    }
    onClose();
  };

  useEffect(() => {
    if (selectedDetectedProfile) {
      const profile = detectedProfiles.find(
        (p) => p.path === selectedDetectedProfile,
      );
      if (profile) {
        const browserName = getBrowserDisplayName(profile.browser);
        const defaultName = `Old ${browserName}`;
        setAutoDetectProfileName(defaultName);
      }
    }
  }, [selectedDetectedProfile, detectedProfiles]);

  const currentMappedBrowser = useMemo(() => {
    if (importMode === "auto-detect" && selectedProfile) {
      return selectedProfile.mapped_browser as "camoufox" | "wayfern";
    }
    if (importMode === "manual" && manualBrowserType) {
      return manualBrowserType as "camoufox" | "wayfern";
    }
    return null;
  }, [importMode, selectedProfile, manualBrowserType]);

  const canProceedToNext = useMemo(() => {
    if (importMode === "auto-detect") {
      return (
        !isLoading &&
        !!selectedDetectedProfile &&
        !!autoDetectProfileName.trim()
      );
    }
    return (
      !!manualBrowserType &&
      !!manualProfilePath.trim() &&
      !!manualProfileName.trim()
    );
  }, [
    importMode,
    isLoading,
    selectedDetectedProfile,
    autoDetectProfileName,
    manualBrowserType,
    manualProfilePath,
    manualProfileName,
  ]);

  useEffect(() => {
    if (isOpen) {
      void loadDetectedProfiles();
    }
  }, [isOpen, loadDetectedProfiles]);

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-2xl max-h-[80vh] my-8 flex flex-col">
        <DialogHeader className="flex-shrink-0">
          <DialogTitle>{t("importProfile.title")}</DialogTitle>
        </DialogHeader>

        <div className="overflow-y-auto flex-1 space-y-6 min-h-0">
          {currentStep === "select" && (
            <>
              <div className="flex gap-2">
                <RippleButton
                  variant={importMode === "auto-detect" ? "default" : "outline"}
                  onClick={() => {
                    setImportMode("auto-detect");
                  }}
                  className="flex-1"
                  disabled={isLoading}
                >
                  {t("importProfile.autoDetect")}
                </RippleButton>
                <RippleButton
                  variant={importMode === "manual" ? "default" : "outline"}
                  onClick={() => {
                    setImportMode("manual");
                  }}
                  className="flex-1"
                  disabled={isLoading}
                >
                  {t("importProfile.manualImport")}
                </RippleButton>
              </div>

              {importMode === "auto-detect" && (
                <div className="space-y-4">
                  <h3 className="text-lg font-medium">
                    {t("importProfile.detectedProfilesTitle")}
                  </h3>

                  {isLoading ? (
                    <div className="py-8 text-center">
                      <p className="text-muted-foreground">
                        {t("importProfile.scanning")}
                      </p>
                    </div>
                  ) : detectedProfiles.length === 0 ? (
                    <div className="py-8 text-center">
                      <p className="text-muted-foreground">
                        {t("importProfile.noneFound")}
                      </p>
                      <p className="mt-2 text-sm text-muted-foreground">
                        {t("importProfile.noneFoundHint")}
                      </p>
                    </div>
                  ) : (
                    <div className="space-y-4">
                      <div>
                        <Label
                          htmlFor="detected-profile-select"
                          className="mb-2"
                        >
                          {t("importProfile.selectProfile")}
                        </Label>
                        <Select
                          value={selectedDetectedProfile ?? undefined}
                          onValueChange={(value) => {
                            setSelectedDetectedProfile(value);
                          }}
                        >
                          <SelectTrigger id="detected-profile-select">
                            <SelectValue
                              placeholder={t(
                                "importProfile.selectProfilePlaceholder",
                              )}
                            />
                          </SelectTrigger>
                          <SelectContent>
                            {detectedProfiles.map((profile) => {
                              const IconComponent = getBrowserIcon(
                                profile.browser,
                              );
                              return (
                                <SelectItem
                                  key={profile.path}
                                  value={profile.path}
                                >
                                  <div className="flex gap-2 items-center">
                                    {IconComponent && (
                                      <IconComponent className="w-4 h-4" />
                                    )}
                                    <div className="flex flex-col">
                                      <span className="font-medium">
                                        {profile.name}
                                      </span>
                                    </div>
                                    <span className="text-xs text-muted-foreground">
                                      →{" "}
                                      {getBrowserDisplayName(
                                        profile.mapped_browser,
                                      )}
                                    </span>
                                  </div>
                                </SelectItem>
                              );
                            })}
                          </SelectContent>
                        </Select>
                      </div>

                      {selectedProfile && (
                        <div className="p-3 rounded-lg bg-muted">
                          <p className="text-sm">
                            <span className="font-medium">
                              {t("importProfile.pathLabel")}
                            </span>{" "}
                            {selectedProfile.path}
                          </p>
                          <p className="text-sm">
                            <span className="font-medium">
                              {t("importProfile.browserLabel")}
                            </span>{" "}
                            {getBrowserDisplayName(selectedProfile.browser)}
                          </p>
                        </div>
                      )}

                      <div>
                        <Label htmlFor="auto-profile-name" className="mb-2">
                          {t("importProfile.newProfileName")}
                        </Label>
                        <Input
                          id="auto-profile-name"
                          value={autoDetectProfileName}
                          onChange={(e) => {
                            setAutoDetectProfileName(e.target.value);
                          }}
                          placeholder={t(
                            "importProfile.newProfileNamePlaceholder",
                          )}
                        />
                      </div>
                    </div>
                  )}
                </div>
              )}

              {importMode === "manual" && (
                <div className="space-y-4">
                  <h3 className="text-lg font-medium">
                    {t("importProfile.manualTitle")}
                  </h3>

                  <div className="space-y-4">
                    <div>
                      <Label htmlFor="manual-browser-select" className="mb-2">
                        {t("importProfile.browserType")}
                      </Label>
                      <Select
                        value={manualBrowserType ?? undefined}
                        onValueChange={(value) => {
                          setManualBrowserType(value);
                        }}
                        disabled={isLoadingSupport}
                      >
                        <SelectTrigger id="manual-browser-select">
                          <SelectValue
                            placeholder={
                              isLoadingSupport
                                ? t("importProfile.loadingBrowsers")
                                : t("importProfile.selectBrowserType")
                            }
                          />
                        </SelectTrigger>
                        <SelectContent>
                          {importableBrowsers.map((browser) => {
                            const IconComponent = getBrowserIcon(browser);
                            return (
                              <SelectItem key={browser} value={browser}>
                                <div className="flex gap-2 items-center">
                                  {IconComponent && (
                                    <IconComponent className="w-4 h-4" />
                                  )}
                                  <span>{getBrowserDisplayName(browser)}</span>
                                </div>
                              </SelectItem>
                            );
                          })}
                        </SelectContent>
                      </Select>
                    </div>

                    <div>
                      <Label htmlFor="manual-profile-path" className="mb-2">
                        {t("importProfile.profileFolderPath")}
                      </Label>
                      <div className="flex gap-2">
                        <Input
                          id="manual-profile-path"
                          value={manualProfilePath}
                          onChange={(e) => {
                            setManualProfilePath(e.target.value);
                          }}
                          placeholder={t(
                            "importProfile.profileFolderPlaceholder",
                          )}
                        />
                        <Button
                          variant="outline"
                          size="icon"
                          onClick={() => void handleBrowseFolder()}
                          title={t("importProfile.browseFolderTitle")}
                        >
                          <FaFolder className="w-4 h-4" />
                        </Button>
                      </div>
                      <p className="mt-2 text-xs text-muted-foreground">
                        {t("importProfile.examplePaths")}
                        <br />
                        macOS: ~/Library/Application
                        Support/Firefox/Profiles/xxx.default
                        <br />
                        Windows: %APPDATA%\Mozilla\Firefox\Profiles\xxx.default
                        <br />
                        Linux: ~/.mozilla/firefox/xxx.default
                      </p>
                    </div>

                    <div>
                      <Label htmlFor="manual-profile-name" className="mb-2">
                        {t("importProfile.newProfileName")}
                      </Label>
                      <Input
                        id="manual-profile-name"
                        value={manualProfileName}
                        onChange={(e) => {
                          setManualProfileName(e.target.value);
                        }}
                        placeholder={t(
                          "importProfile.newProfileNamePlaceholder",
                        )}
                      />
                    </div>
                  </div>
                </div>
              )}
            </>
          )}

          {currentStep === "configure" && currentMappedBrowser && (
            <div className="space-y-4">
              <Alert>
                <AlertDescription>
                  {t("importProfile.importedAsPrefix")}{" "}
                  <strong>{getBrowserDisplayName(currentMappedBrowser)}</strong>{" "}
                  {t("importProfile.importedAsSuffix")}
                </AlertDescription>
              </Alert>

              <div>
                <Label className="mb-2">
                  {t("importProfile.proxyOptional")}
                </Label>
                <Select
                  value={selectedProxyId ?? "none"}
                  onValueChange={(value) => {
                    setSelectedProxyId(value === "none" ? undefined : value);
                  }}
                >
                  <SelectTrigger>
                    <SelectValue placeholder={t("importProfile.noProxy")} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="none">
                      {t("importProfile.noProxy")}
                    </SelectItem>
                    {storedProxies.map((proxy) => (
                      <SelectItem key={proxy.id} value={proxy.id}>
                        {proxy.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              {currentMappedBrowser === "camoufox" ? (
                <SharedCamoufoxConfigForm
                  config={camoufoxConfig}
                  onConfigChange={(key, value) => {
                    setCamoufoxConfig((prev) => ({ ...prev, [key]: value }));
                  }}
                  isCreating={true}
                  crossOsUnlocked={crossOsUnlocked}
                  limitedMode={!crossOsUnlocked}
                />
              ) : (
                <WayfernConfigForm
                  config={wayfernConfig}
                  onConfigChange={(key, value) => {
                    setWayfernConfig((prev) => ({ ...prev, [key]: value }));
                  }}
                  isCreating={true}
                  crossOsUnlocked={crossOsUnlocked}
                  limitedMode={!crossOsUnlocked}
                />
              )}
            </div>
          )}
        </div>

        <DialogFooter className="flex-shrink-0">
          {currentStep === "select" ? (
            <>
              <RippleButton variant="outline" onClick={handleClose}>
                {t("common.buttons.cancel")}
              </RippleButton>
              <RippleButton
                disabled={!canProceedToNext}
                onClick={() => {
                  setCurrentStep("configure");
                }}
              >
                {t("importProfile.nextButton")}
              </RippleButton>
            </>
          ) : (
            <>
              <RippleButton
                variant="outline"
                onClick={() => {
                  setCurrentStep("select");
                }}
              >
                {t("common.buttons.back")}
              </RippleButton>
              <LoadingButton
                isLoading={isImporting}
                onClick={() => {
                  void handleImport();
                }}
              >
                {t("importProfile.importButton")}
              </LoadingButton>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
