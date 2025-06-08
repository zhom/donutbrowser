"use client";

import { LoadingButton } from "@/components/loading-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { usePermissions } from "@/hooks/use-permissions";
import type { PermissionType } from "@/hooks/use-permissions";
import { showSuccessToast } from "@/lib/toast-utils";
import { invoke } from "@tauri-apps/api/core";
import { useTheme } from "next-themes";
import { useCallback, useEffect, useState } from "react";
import { BsCamera, BsMic } from "react-icons/bs";

interface AppSettings {
  set_as_default_browser: boolean;
  show_settings_on_startup: boolean;
  theme: string;
  auto_updates_enabled: boolean;
  auto_delete_unused_binaries: boolean;
}

interface PermissionInfo {
  permission_type: PermissionType;
  isGranted: boolean;
  description: string;
}

interface SettingsDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function SettingsDialog({ isOpen, onClose }: SettingsDialogProps) {
  const [settings, setSettings] = useState<AppSettings>({
    set_as_default_browser: false,
    show_settings_on_startup: true,
    theme: "system",
    auto_updates_enabled: true,
    auto_delete_unused_binaries: true,
  });
  const [originalSettings, setOriginalSettings] = useState<AppSettings>({
    set_as_default_browser: false,
    show_settings_on_startup: true,
    theme: "system",
    auto_updates_enabled: true,
    auto_delete_unused_binaries: true,
  });
  const [isDefaultBrowser, setIsDefaultBrowser] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [isSettingDefault, setIsSettingDefault] = useState(false);
  const [isClearingCache, setIsClearingCache] = useState(false);
  const [permissions, setPermissions] = useState<PermissionInfo[]>([]);
  const [isLoadingPermissions, setIsLoadingPermissions] = useState(false);
  const [requestingPermission, setRequestingPermission] =
    useState<PermissionType | null>(null);
  const [isMacOS, setIsMacOS] = useState(false);

  const { setTheme } = useTheme();
  const {
    requestPermission,
    isMicrophoneAccessGranted,
    isCameraAccessGranted,
  } = usePermissions();

  const getPermissionDescription = useCallback((type: PermissionType) => {
    switch (type) {
      case "microphone":
        return "Access to microphone for browser applications";
      case "camera":
        return "Access to camera for browser applications";
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      loadSettings().catch(console.error);
      checkDefaultBrowserStatus().catch(console.error);

      // Check if we're on macOS
      const userAgent = navigator.userAgent;
      const isMac = userAgent.includes("Mac");
      setIsMacOS(isMac);

      if (isMac) {
        loadPermissions().catch(console.error);
      }

      // Set up interval to check default browser status
      const intervalId = setInterval(() => {
        checkDefaultBrowserStatus().catch(console.error);
      }, 500); // Check every 500ms

      // Cleanup interval on component unmount or dialog close
      return () => {
        clearInterval(intervalId);
      };
    }
  }, [isOpen]);

  // Update permissions when the permission states change
  useEffect(() => {
    if (isMacOS) {
      const permissionList: PermissionInfo[] = [
        {
          permission_type: "microphone",
          isGranted: isMicrophoneAccessGranted,
          description: getPermissionDescription("microphone"),
        },
        {
          permission_type: "camera",
          isGranted: isCameraAccessGranted,
          description: getPermissionDescription("camera"),
        },
      ];
      setPermissions(permissionList);
    } else {
      setPermissions([]);
    }
  }, [
    isMacOS,
    isMicrophoneAccessGranted,
    isCameraAccessGranted,
    getPermissionDescription,
  ]);

  const loadSettings = async () => {
    setIsLoading(true);
    try {
      const appSettings = await invoke<AppSettings>("get_app_settings");
      setSettings(appSettings);
      setOriginalSettings(appSettings);
    } catch (error) {
      console.error("Failed to load settings:", error);
    } finally {
      setIsLoading(false);
    }
  };

  const loadPermissions = async () => {
    setIsLoadingPermissions(true);
    try {
      if (!isMacOS) {
        // On non-macOS platforms, don't show permissions
        setPermissions([]);
        return;
      }

      const permissionList: PermissionInfo[] = [
        {
          permission_type: "microphone",
          isGranted: isMicrophoneAccessGranted,
          description: getPermissionDescription("microphone"),
        },
        {
          permission_type: "camera",
          isGranted: isCameraAccessGranted,
          description: getPermissionDescription("camera"),
        },
      ];

      setPermissions(permissionList);
    } catch (error) {
      console.error("Failed to load permissions:", error);
    } finally {
      setIsLoadingPermissions(false);
    }
  };

  const checkDefaultBrowserStatus = async () => {
    try {
      const isDefault = await invoke<boolean>("is_default_browser");
      setIsDefaultBrowser(isDefault);
    } catch (error) {
      console.error("Failed to check default browser status:", error);
    }
  };

  const handleSetDefaultBrowser = async () => {
    setIsSettingDefault(true);
    try {
      await invoke("set_as_default_browser");
      await checkDefaultBrowserStatus();
    } catch (error) {
      console.error("Failed to set as default browser:", error);
    } finally {
      setIsSettingDefault(false);
    }
  };

  const handleClearCache = async () => {
    setIsClearingCache(true);
    try {
      await invoke("clear_all_version_cache_and_refetch");
      showSuccessToast("Cache cleared successfully", {
        description:
          "All browser version cache has been cleared and browsers are being refreshed",
        duration: 4000,
      });
    } catch (error) {
      console.error("Failed to clear cache:", error);
    } finally {
      setIsClearingCache(false);
    }
  };

  const handleRequestPermission = async (permissionType: PermissionType) => {
    setRequestingPermission(permissionType);
    try {
      await requestPermission(permissionType);
      showSuccessToast(
        `${getPermissionDisplayName(permissionType)} access requested`,
      );
    } catch (error) {
      console.error("Failed to request permission:", error);
    } finally {
      setRequestingPermission(null);
    }
  };

  const getPermissionIcon = (type: PermissionType) => {
    switch (type) {
      case "microphone":
        return <BsMic className="h-4 w-4" />;
      case "camera":
        return <BsCamera className="h-4 w-4" />;
    }
  };

  const getPermissionDisplayName = (type: PermissionType) => {
    switch (type) {
      case "microphone":
        return "Microphone";
      case "camera":
        return "Camera";
    }
  };

  const getStatusBadge = (isGranted: boolean) => {
    if (isGranted) {
      return (
        <Badge variant="default" className="bg-green-100 text-green-800">
          Granted
        </Badge>
      );
    }
    return <Badge variant="secondary">Not Granted</Badge>;
  };

  const handleSave = async () => {
    setIsSaving(true);
    try {
      await invoke("save_app_settings", { settings });
      setTheme(settings.theme);
      setOriginalSettings(settings);
      onClose();
    } catch (error) {
      console.error("Failed to save settings:", error);
    } finally {
      setIsSaving(false);
    }
  };

  const updateSetting = (key: keyof AppSettings, value: boolean | string) => {
    setSettings((prev) => ({ ...prev, [key]: value }));
  };

  // Check if settings have changed (excluding default browser setting)
  const hasChanges =
    settings.show_settings_on_startup !==
      originalSettings.show_settings_on_startup ||
    settings.theme !== originalSettings.theme ||
    settings.auto_updates_enabled !== originalSettings.auto_updates_enabled ||
    settings.auto_delete_unused_binaries !==
      originalSettings.auto_delete_unused_binaries;

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md max-h-[80vh] my-8 flex flex-col">
        <DialogHeader className="flex-shrink-0">
          <DialogTitle>Settings</DialogTitle>
        </DialogHeader>

        <div className="grid overflow-y-auto flex-1 gap-6 py-4 min-h-0">
          {/* Appearance Section */}
          <div className="space-y-4">
            <Label className="text-base font-medium">Appearance</Label>

            <div className="grid gap-2">
              <Label htmlFor="theme-select" className="text-sm">
                Theme
              </Label>
              <Select
                value={settings.theme}
                onValueChange={(value) => {
                  updateSetting("theme", value);
                }}
              >
                <SelectTrigger id="theme-select">
                  <SelectValue placeholder="Select theme" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="light">Light</SelectItem>
                  <SelectItem value="dark">Dark</SelectItem>
                  <SelectItem value="system">System</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <p className="text-xs text-muted-foreground">
              Choose your preferred theme or follow your system settings.
            </p>
          </div>

          {/* Default Browser Section */}
          <div className="space-y-4">
            <div className="flex justify-between items-center">
              <Label className="text-base font-medium">Default Browser</Label>
              <Badge variant={isDefaultBrowser ? "default" : "secondary"}>
                {isDefaultBrowser ? "Active" : "Inactive"}
              </Badge>
            </div>

            <LoadingButton
              isLoading={isSettingDefault}
              onClick={() => {
                handleSetDefaultBrowser().catch(console.error);
              }}
              disabled={isDefaultBrowser}
              variant={isDefaultBrowser ? "outline" : "default"}
              className="w-full"
            >
              {isDefaultBrowser
                ? "Already Default Browser"
                : "Set as Default Browser"}
            </LoadingButton>

            <p className="text-xs text-muted-foreground">
              When set as default, Donut Browser will handle web links and allow
              you to choose which profile to use.
            </p>
          </div>

          {/* Auto-Update Section */}
          <div className="space-y-4">
            <Label className="text-base font-medium">Auto-Updates</Label>

            <div className="flex items-center space-x-2">
              <Checkbox
                id="auto-updates"
                checked={settings.auto_updates_enabled}
                onCheckedChange={(checked) => {
                  updateSetting("auto_updates_enabled", checked as boolean);
                }}
              />
              <Label htmlFor="auto-updates" className="text-sm">
                Enable automatic browser updates
              </Label>
            </div>

            <div className="flex items-center space-x-2">
              <Checkbox
                id="auto-delete-binaries"
                checked={settings.auto_delete_unused_binaries}
                onCheckedChange={(checked) => {
                  updateSetting(
                    "auto_delete_unused_binaries",
                    checked as boolean,
                  );
                }}
              />
              <Label htmlFor="auto-delete-binaries" className="text-sm">
                Automatically delete unused browser binaries
              </Label>
            </div>

            <p className="text-xs text-muted-foreground">
              When enabled, Donut Browser will check for browser updates and
              notify you when updates are available for your profiles. Unused
              binaries will be automatically deleted to save disk space.
            </p>
          </div>

          {/* Startup Behavior Section */}
          <div className="space-y-4">
            <Label className="text-base font-medium">Startup Behavior</Label>

            <div className="flex items-center space-x-2">
              <Checkbox
                id="show-settings"
                checked={settings.show_settings_on_startup}
                onCheckedChange={(checked) => {
                  updateSetting("show_settings_on_startup", checked as boolean);
                }}
              />
              <Label htmlFor="show-settings" className="text-sm">
                Show settings on app startup
              </Label>
            </div>

            <p className="text-xs text-muted-foreground">
              When enabled, the settings dialog will be shown when the app
              starts.
            </p>
          </div>

          {/* Permissions Section - Only show on macOS */}
          {isMacOS && (
            <div className="space-y-4">
              <Label className="text-base font-medium">
                System Permissions
              </Label>

              {isLoadingPermissions ? (
                <div className="text-sm text-muted-foreground">
                  Loading permissions...
                </div>
              ) : (
                <div className="space-y-3">
                  {permissions.map((permission) => (
                    <div
                      key={permission.permission_type}
                      className="flex items-center justify-between p-3 border rounded-lg"
                    >
                      <div className="flex items-center space-x-3">
                        {getPermissionIcon(permission.permission_type)}
                        <div>
                          <div className="text-sm font-medium">
                            {getPermissionDisplayName(
                              permission.permission_type,
                            )}
                          </div>
                          <div className="text-xs text-muted-foreground">
                            {permission.description}
                          </div>
                        </div>
                      </div>
                      <div className="flex items-center space-x-2">
                        {getStatusBadge(permission.isGranted)}
                        {!permission.isGranted && (
                          <LoadingButton
                            size="sm"
                            isLoading={
                              requestingPermission ===
                              permission.permission_type
                            }
                            onClick={() => {
                              handleRequestPermission(
                                permission.permission_type,
                              ).catch(console.error);
                            }}
                          >
                            Grant
                          </LoadingButton>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              )}

              <p className="text-xs text-muted-foreground">
                These permissions allow browsers launched from Donut Browser to
                access system resources. Each website will still ask for your
                permission individually.
              </p>
            </div>
          )}

          {/* Advanced Section */}
          <div className="space-y-4">
            <Label className="text-base font-medium">Advanced</Label>

            <LoadingButton
              isLoading={isClearingCache}
              onClick={() => {
                handleClearCache().catch(console.error);
              }}
              variant="outline"
              className="w-full"
            >
              Clear All Version Cache
            </LoadingButton>

            <p className="text-xs text-muted-foreground">
              Clear all cached browser version data and refresh all browser
              versions from their sources. This will force a fresh download of
              version information for all browsers.
            </p>
          </div>
        </div>

        <DialogFooter className="flex-shrink-0">
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <LoadingButton
            isLoading={isSaving}
            onClick={() => {
              handleSave().catch(console.error);
            }}
            disabled={isLoading || !hasChanges}
          >
            Save Settings
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
