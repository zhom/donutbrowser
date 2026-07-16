"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { FaFileArchive, FaFolder } from "react-icons/fa";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Progress } from "@/components/ui/progress";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { WayfernConfigForm } from "@/components/wayfern-config-form";
import { useGroupEvents } from "@/hooks/use-group-events";
import { useProxyEvents } from "@/hooks/use-proxy-events";
import { translateBackendError } from "@/lib/backend-errors";
import { getBrowserDisplayName, getBrowserIcon } from "@/lib/browser-utils";
import { cn } from "@/lib/utils";
import type {
  ArchiveScanResult,
  DetectedProfile,
  ImportProfileItem,
  ProfileImportBatchResult,
  ProfileImportProgress,
  WayfernConfig,
} from "@/types";
import { RippleButton } from "./ui/ripple";

interface ImportProfileDialogProps {
  isOpen: boolean;
  onClose: () => void;
  crossOsUnlocked?: boolean;
  subPage?: boolean;
}

type Step = "select" | "configure" | "importing";
type ImportMode = "auto-detect" | "manual";
type DuplicateStrategy = "rename" | "skip";

export function ImportProfileDialog({
  isOpen,
  onClose,
  crossOsUnlocked,
  subPage,
}: ImportProfileDialogProps) {
  const { t } = useTranslation();
  const [currentStep, setCurrentStep] = useState<Step>("select");
  const [importMode, setImportMode] = useState<ImportMode>("auto-detect");

  const [detectedProfiles, setDetectedProfiles] = useState<DetectedProfile[]>(
    [],
  );
  const [scannedProfiles, setScannedProfiles] = useState<DetectedProfile[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isScanning, setIsScanning] = useState(false);
  const [manualPath, setManualPath] = useState("");
  const [extractedDir, setExtractedDir] = useState<string | null>(null);

  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [profileNames, setProfileNames] = useState<Record<string, string>>({});

  const [selectedGroupId, setSelectedGroupId] = useState<string>("none");
  const [isCreatingGroup, setIsCreatingGroup] = useState(false);
  const [newGroupName, setNewGroupName] = useState("");
  const [duplicateStrategy, setDuplicateStrategy] =
    useState<DuplicateStrategy>("rename");
  // "none" | "round-robin" | a stored proxy id
  const [proxyAssignment, setProxyAssignment] = useState<string>("none");
  const [wayfernConfig, setWayfernConfig] = useState<WayfernConfig>({});

  const [isImporting, setIsImporting] = useState(false);
  const [progress, setProgress] = useState<ProfileImportProgress | null>(null);
  const [result, setResult] = useState<ProfileImportBatchResult | null>(null);

  const { storedProxies } = useProxyEvents();
  const { groups } = useGroupEvents();

  const activeProfiles =
    importMode === "auto-detect" ? detectedProfiles : scannedProfiles;
  const selectedProfiles = useMemo(
    () => activeProfiles.filter((p) => selectedPaths.has(p.path)),
    [activeProfiles, selectedPaths],
  );

  const registerProfileNames = useCallback((profiles: DetectedProfile[]) => {
    setProfileNames((prev) => {
      const next = { ...prev };
      for (const profile of profiles) {
        if (next[profile.path] === undefined) {
          next[profile.path] = profile.name;
        }
      }
      return next;
    });
  }, []);

  const loadDetectedProfiles = useCallback(async () => {
    setIsLoading(true);
    try {
      const profiles = await invoke<DetectedProfile[]>(
        "detect_existing_profiles",
      );
      setDetectedProfiles(profiles);
      registerProfileNames(profiles);
      if (profiles.length === 0) {
        setImportMode("manual");
      }
    } catch (error) {
      console.error("Failed to detect existing profiles:", error);
      toast.error(t("importProfile.detectFailed"));
    } finally {
      setIsLoading(false);
    }
  }, [t, registerProfileNames]);

  const cleanupExtractedDir = useCallback(async (dir: string | null) => {
    if (!dir) return;
    try {
      await invoke("cleanup_profile_import_scratch", { extractedDir: dir });
    } catch (error) {
      console.error("Failed to clean up extracted archive:", error);
    }
  }, []);

  const applyScanResult = useCallback(
    (profiles: DetectedProfile[]) => {
      setScannedProfiles(profiles);
      registerProfileNames(profiles);
      setSelectedPaths(new Set(profiles.map((p) => p.path)));
      if (profiles.length === 0) {
        toast.info(t("importProfile.noProfilesInLocation"));
      }
    },
    [registerProfileNames, t],
  );

  const scanPath = useCallback(
    async (path: string) => {
      setIsScanning(true);
      try {
        if (path.toLowerCase().endsWith(".zip")) {
          await cleanupExtractedDir(extractedDir);
          setExtractedDir(null);
          const scan = await invoke<ArchiveScanResult>("scan_profile_archive", {
            archivePath: path,
          });
          setExtractedDir(scan.extracted_dir);
          applyScanResult(scan.profiles);
        } else {
          const profiles = await invoke<DetectedProfile[]>(
            "scan_folder_for_profiles",
            { folderPath: path },
          );
          applyScanResult(profiles);
        }
      } catch (error) {
        console.error("Failed to scan for profiles:", error);
        toast.error(translateBackendError(t, error));
      } finally {
        setIsScanning(false);
      }
    },
    [applyScanResult, cleanupExtractedDir, extractedDir, t],
  );

  const handleBrowseFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: t("importProfile.selectFolderTitle"),
      });
      if (selected && typeof selected === "string") {
        setManualPath(selected);
        await scanPath(selected);
      }
    } catch (error) {
      console.error("Failed to open folder dialog:", error);
      toast.error(t("importProfile.folderDialogFailed"));
    }
  };

  const handleBrowseArchive = async () => {
    try {
      const selected = await open({
        multiple: false,
        title: t("importProfile.selectArchiveTitle"),
        filters: [{ name: "ZIP", extensions: ["zip"] }],
      });
      if (selected && typeof selected === "string") {
        setManualPath(selected);
        await scanPath(selected);
      }
    } catch (error) {
      console.error("Failed to open archive dialog:", error);
      toast.error(t("importProfile.folderDialogFailed"));
    }
  };

  const togglePath = (path: string, checked: boolean) => {
    setSelectedPaths((prev) => {
      const next = new Set(prev);
      if (checked) {
        next.add(path);
      } else {
        next.delete(path);
      }
      return next;
    });
  };

  const toggleAll = (checked: boolean) => {
    setSelectedPaths(
      checked ? new Set(activeProfiles.map((p) => p.path)) : new Set(),
    );
  };

  const proxyIdForIndex = useCallback(
    (index: number): string | null => {
      if (proxyAssignment === "none") return null;
      if (proxyAssignment === "round-robin") {
        if (storedProxies.length === 0) return null;
        return storedProxies[index % storedProxies.length].id;
      }
      return proxyAssignment;
    },
    [proxyAssignment, storedProxies],
  );

  const handleCreateGroup = async () => {
    const name = newGroupName.trim();
    if (!name) return;
    try {
      const group = await invoke<{ id: string; name: string }>(
        "create_profile_group",
        { name },
      );
      setSelectedGroupId(group.id);
      setIsCreatingGroup(false);
      setNewGroupName("");
    } catch (error) {
      console.error("Failed to create group:", error);
      toast.error(translateBackendError(t, error));
    }
  };

  const handleImport = useCallback(async () => {
    if (selectedProfiles.length === 0) {
      toast.error(t("importProfile.selectAtLeastOne"));
      return;
    }
    if (
      selectedProfiles.some((p) => !(profileNames[p.path] ?? p.name).trim())
    ) {
      toast.error(t("importProfile.emptyNames"));
      return;
    }

    const items: ImportProfileItem[] = selectedProfiles.map((p, index) => ({
      source_path: p.path,
      browser_type: p.browser,
      new_profile_name: (profileNames[p.path] ?? p.name).trim(),
      proxy_id: proxyIdForIndex(index),
    }));

    setCurrentStep("importing");
    setIsImporting(true);
    setProgress(null);
    setResult(null);
    try {
      const batchResult = await invoke<ProfileImportBatchResult>(
        "import_browser_profiles",
        {
          items,
          groupId: selectedGroupId === "none" ? null : selectedGroupId,
          duplicateStrategy: duplicateStrategy,
          wayfernConfig,
        },
      );
      setResult(batchResult);
      toast.success(
        t("importProfile.resultsSummary", {
          imported: batchResult.imported_count,
          skipped: batchResult.skipped_count,
          failed: batchResult.failed_count,
        }),
      );
    } catch (error) {
      console.error("Failed to import profiles:", error);
      toast.error(translateBackendError(t, error));
      setCurrentStep("configure");
    } finally {
      setIsImporting(false);
    }
  }, [
    selectedProfiles,
    profileNames,
    proxyIdForIndex,
    selectedGroupId,
    duplicateStrategy,
    wayfernConfig,
    t,
  ]);

  const handleClose = () => {
    void cleanupExtractedDir(extractedDir);
    setCurrentStep("select");
    setImportMode(detectedProfiles.length > 0 ? "auto-detect" : "manual");
    setScannedProfiles([]);
    setManualPath("");
    setExtractedDir(null);
    setSelectedPaths(new Set());
    setProfileNames({});
    setSelectedGroupId("none");
    setIsCreatingGroup(false);
    setNewGroupName("");
    setDuplicateStrategy("rename");
    setProxyAssignment("none");
    setWayfernConfig({});
    setProgress(null);
    setResult(null);
    onClose();
  };

  useEffect(() => {
    if (isOpen) {
      void loadDetectedProfiles();
    }
  }, [isOpen, loadDetectedProfiles]);

  useEffect(() => {
    if (!isOpen) return;
    const unlistenPromise = listen<ProfileImportProgress>(
      "profile-import-progress",
      (event) => {
        setProgress(event.payload);
      },
    );
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [isOpen]);

  const allSelected =
    activeProfiles.length > 0 && selectedPaths.size >= activeProfiles.length;

  const renderProfileList = (profiles: DetectedProfile[]) => (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <label
          htmlFor="import-select-all"
          className="flex cursor-pointer items-center gap-2 text-sm"
        >
          <Checkbox
            id="import-select-all"
            checked={allSelected}
            onCheckedChange={(checked) => toggleAll(checked === true)}
          />
          {t("importProfile.selectAll")}
        </label>
        <span className="text-xs text-muted-foreground">
          {t("importProfile.selectedCount", { count: selectedPaths.size })}
        </span>
      </div>
      <div className="max-h-64 space-y-1 overflow-y-auto rounded-lg border border-border p-2">
        {profiles.map((profile) => {
          const IconComponent = getBrowserIcon(profile.browser);
          const checkboxId = `import-profile-${encodeURIComponent(profile.path)}`;
          return (
            <label
              key={profile.path}
              htmlFor={checkboxId}
              className="flex cursor-pointer items-center gap-2 rounded-md p-2 hover:bg-muted"
            >
              <Checkbox
                id={checkboxId}
                checked={selectedPaths.has(profile.path)}
                onCheckedChange={(checked) =>
                  togglePath(profile.path, checked === true)
                }
              />
              {IconComponent && <IconComponent className="size-4 shrink-0" />}
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium">{profile.name}</p>
                <p className="truncate text-xs text-muted-foreground">
                  {profile.path}
                </p>
              </div>
            </label>
          );
        })}
      </div>
    </div>
  );

  const progressPercent =
    progress && progress.total > 0
      ? Math.round((progress.completed / progress.total) * 100)
      : 0;

  return (
    <Dialog open={isOpen} onOpenChange={handleClose} subPage={subPage}>
      <DialogContent className="flex max-h-[80vh] max-w-[min(48rem,calc(100%-4rem))] flex-col">
        {!subPage && (
          <DialogHeader className="shrink-0">
            <DialogTitle>{t("importProfile.title")}</DialogTitle>
          </DialogHeader>
        )}

        <div
          className={cn(
            "min-h-0 flex-1 space-y-6 overflow-y-auto",
            subPage && "mx-auto w-full max-w-2xl",
          )}
        >
          {currentStep === "select" && (
            <AnimatedTabs
              value={importMode}
              onValueChange={(v) => {
                setImportMode(v as ImportMode);
                setSelectedPaths(new Set());
              }}
              className="flex flex-col gap-6"
            >
              <AnimatedTabsList>
                <AnimatedTabsTrigger value="auto-detect" disabled={isLoading}>
                  {t("importProfile.autoDetect")}
                </AnimatedTabsTrigger>
                <AnimatedTabsTrigger value="manual" disabled={isLoading}>
                  {t("importProfile.manualImport")}
                </AnimatedTabsTrigger>
              </AnimatedTabsList>

              <AnimatedTabsContent value="auto-detect">
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
                    renderProfileList(detectedProfiles)
                  )}
                </div>
              </AnimatedTabsContent>

              <AnimatedTabsContent value="manual">
                <div className="space-y-4">
                  <h3 className="text-lg font-medium">
                    {t("importProfile.manualTitle")}
                  </h3>

                  <div>
                    <Label htmlFor="manual-profile-path" className="mb-2">
                      {t("importProfile.profileFolderPath")}
                    </Label>
                    <div className="flex gap-2">
                      <Input
                        id="manual-profile-path"
                        value={manualPath}
                        onChange={(e) => {
                          setManualPath(e.target.value);
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
                        <FaFolder className="size-4" />
                      </Button>
                      <Button
                        variant="outline"
                        size="icon"
                        onClick={() => void handleBrowseArchive()}
                        title={t("importProfile.selectArchiveTitle")}
                      >
                        <FaFileArchive className="size-4" />
                      </Button>
                      <LoadingButton
                        variant="outline"
                        isLoading={isScanning}
                        disabled={!manualPath.trim()}
                        onClick={() => void scanPath(manualPath.trim())}
                      >
                        {t("importProfile.scanButton")}
                      </LoadingButton>
                    </div>
                    <p className="mt-2 text-xs text-muted-foreground">
                      {t("importProfile.manualHint")}
                    </p>
                    <p className="mt-2 text-xs break-all text-muted-foreground">
                      {t("importProfile.examplePaths")}
                      <br />
                      macOS: ~/Library/Application Support/Google/Chrome/Default
                      <br />
                      Windows: %LOCALAPPDATA%\Google\Chrome\User Data\Default
                      <br />
                      Linux: ~/.config/google-chrome/Default
                    </p>
                  </div>

                  {scannedProfiles.length > 0 &&
                    renderProfileList(scannedProfiles)}
                </div>
              </AnimatedTabsContent>
            </AnimatedTabs>
          )}

          {currentStep === "configure" && (
            <div className="space-y-4">
              <Alert>
                <AlertDescription>
                  {t("importProfile.importedAs", {
                    browser: getBrowserDisplayName("wayfern"),
                  })}
                </AlertDescription>
              </Alert>

              <div>
                <Label className="mb-2">
                  {t("importProfile.profilesToImport")}
                </Label>
                <div className="max-h-48 space-y-2 overflow-y-auto rounded-lg border border-border p-2">
                  {selectedProfiles.map((profile) => (
                    <div key={profile.path} className="flex items-center gap-2">
                      <span
                        className="min-w-0 flex-1 truncate text-xs text-muted-foreground"
                        title={profile.path}
                      >
                        {profile.name}
                      </span>
                      <Input
                        className="flex-1"
                        aria-label={t("importProfile.newProfileName")}
                        value={profileNames[profile.path] ?? profile.name}
                        onChange={(e) => {
                          setProfileNames((prev) => ({
                            ...prev,
                            [profile.path]: e.target.value,
                          }));
                        }}
                        placeholder={t(
                          "importProfile.newProfileNamePlaceholder",
                        )}
                      />
                    </div>
                  ))}
                </div>
              </div>

              <div>
                <Label className="mb-2">
                  {t("importProfile.groupOptional")}
                </Label>
                {isCreatingGroup ? (
                  <div className="flex gap-2">
                    <Input
                      value={newGroupName}
                      onChange={(e) => setNewGroupName(e.target.value)}
                      placeholder={t("importProfile.newGroupNamePlaceholder")}
                    />
                    <Button
                      variant="outline"
                      disabled={!newGroupName.trim()}
                      onClick={() => void handleCreateGroup()}
                    >
                      {t("common.buttons.create")}
                    </Button>
                    <Button
                      variant="ghost"
                      onClick={() => {
                        setIsCreatingGroup(false);
                        setNewGroupName("");
                      }}
                    >
                      {t("common.buttons.cancel")}
                    </Button>
                  </div>
                ) : (
                  <Select
                    value={selectedGroupId}
                    onValueChange={(value) => {
                      if (value === "create-new") {
                        setIsCreatingGroup(true);
                      } else {
                        setSelectedGroupId(value);
                      }
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue placeholder={t("importProfile.noGroup")} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="none">
                        {t("importProfile.noGroup")}
                      </SelectItem>
                      {groups.map((group) => (
                        <SelectItem key={group.id} value={group.id}>
                          {group.name}
                        </SelectItem>
                      ))}
                      <SelectItem value="create-new">
                        {t("importProfile.createNewGroup")}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                )}
              </div>

              <div>
                <Label className="mb-2">
                  {t("importProfile.duplicateStrategyLabel")}
                </Label>
                <Select
                  value={duplicateStrategy}
                  onValueChange={(value) => {
                    setDuplicateStrategy(value as DuplicateStrategy);
                  }}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="rename">
                      {t("importProfile.duplicateRename")}
                    </SelectItem>
                    <SelectItem value="skip">
                      {t("importProfile.duplicateSkip")}
                    </SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div>
                <Label className="mb-2">
                  {t("importProfile.proxyOptional")}
                </Label>
                <Select
                  value={proxyAssignment}
                  onValueChange={setProxyAssignment}
                >
                  <SelectTrigger>
                    <SelectValue placeholder={t("importProfile.noProxy")} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="none">
                      {t("importProfile.noProxy")}
                    </SelectItem>
                    {storedProxies.length > 0 && (
                      <SelectItem value="round-robin">
                        {t("importProfile.proxyRoundRobin")}
                      </SelectItem>
                    )}
                    {storedProxies.map((proxy) => (
                      <SelectItem key={proxy.id} value={proxy.id}>
                        {proxy.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <WayfernConfigForm
                config={wayfernConfig}
                onConfigChange={(key, value) => {
                  setWayfernConfig((prev) => ({ ...prev, [key]: value }));
                }}
                isCreating={true}
                crossOsUnlocked={crossOsUnlocked}
                limitedMode={!crossOsUnlocked}
              />
            </div>
          )}

          {currentStep === "importing" && (
            <div className="space-y-4">
              {isImporting && (
                <div className="space-y-2">
                  <h3 className="text-lg font-medium">
                    {t("importProfile.importingTitle")}
                  </h3>
                  <Progress value={progressPercent} />
                  {progress && (
                    <p className="text-sm text-muted-foreground">
                      {t("importProfile.importProgress", {
                        completed: progress.completed,
                        total: progress.total,
                      })}
                      {progress.status === "importing" && (
                        <> — {progress.name}</>
                      )}
                    </p>
                  )}
                </div>
              )}

              {result && (
                <div className="space-y-2">
                  <h3 className="text-lg font-medium">
                    {t("importProfile.resultsSummary", {
                      imported: result.imported_count,
                      skipped: result.skipped_count,
                      failed: result.failed_count,
                    })}
                  </h3>
                  <div className="max-h-64 space-y-1 overflow-y-auto rounded-lg border border-border p-2">
                    {result.results.map((item) => (
                      <div
                        key={item.source_path}
                        className="flex items-center gap-2 p-1 text-sm"
                      >
                        <span
                          className={cn(
                            "shrink-0 text-xs font-medium",
                            item.status === "imported" && "text-success",
                            item.status === "skipped" &&
                              "text-muted-foreground",
                            item.status === "failed" && "text-destructive",
                          )}
                        >
                          {item.status === "imported" &&
                            t("importProfile.statusImported")}
                          {item.status === "skipped" &&
                            t("importProfile.statusSkipped")}
                          {item.status === "failed" &&
                            t("importProfile.statusFailed")}
                        </span>
                        <span className="min-w-0 flex-1 truncate">
                          {item.name || item.source_path}
                        </span>
                        {item.error && (
                          <span className="min-w-0 flex-1 truncate text-xs text-destructive">
                            {translateBackendError(t, new Error(item.error))}
                          </span>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>

        <div
          className={cn(
            "flex shrink-0 items-center justify-end gap-2",
            subPage
              ? "mx-auto w-full max-w-2xl border-t border-border pt-2"
              : undefined,
          )}
        >
          {currentStep === "select" && (
            <>
              {!subPage && (
                <RippleButton variant="outline" onClick={handleClose}>
                  {t("common.buttons.cancel")}
                </RippleButton>
              )}
              <RippleButton
                disabled={selectedPaths.size === 0}
                onClick={() => {
                  setCurrentStep("configure");
                }}
              >
                {t("importProfile.nextButton")}
              </RippleButton>
            </>
          )}
          {currentStep === "configure" && (
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
                {t("importProfile.importButtonCount", {
                  count: selectedProfiles.length,
                })}
              </LoadingButton>
            </>
          )}
          {currentStep === "importing" && (
            <RippleButton disabled={isImporting} onClick={handleClose}>
              {t("common.buttons.close")}
            </RippleButton>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
