"use client";

import { invoke } from "@tauri-apps/api/core";
import Color from "color";
import { useTheme } from "next-themes";
import { useCallback, useEffect, useState } from "react";
import { BsCamera, BsMic } from "react-icons/bs";
import { LoadingButton } from "@/components/loading-button";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import {
  ColorPicker,
  ColorPickerAlpha,
  ColorPickerEyeDropper,
  ColorPickerFormat,
  ColorPickerHue,
  ColorPickerOutput,
  ColorPickerSelection,
} from "@/components/ui/color-picker";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useCommercialTrial } from "@/hooks/use-commercial-trial";
import type { PermissionType } from "@/hooks/use-permissions";
import { usePermissions } from "@/hooks/use-permissions";
import { useWayfernTerms } from "@/hooks/use-wayfern-terms";
import {
  getThemeByColors,
  getThemeById,
  THEME_VARIABLES,
  THEMES,
} from "@/lib/themes";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { CopyToClipboard } from "./ui/copy-to-clipboard";
import { RippleButton } from "./ui/ripple";

interface AppSettings {
  set_as_default_browser: boolean;
  theme: string;
  custom_theme?: Record<string, string>;
  api_enabled: boolean;
  api_port: number;
  api_token?: string;
}

interface CustomThemeState {
  selectedThemeId: string | null;
  colors: Record<string, string>;
}

interface PermissionInfo {
  permission_type: PermissionType;
  isGranted: boolean;
  description: string;
}

// Version update progress toasts are handled globally via useVersionUpdater

interface SettingsDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function SettingsDialog({ isOpen, onClose }: SettingsDialogProps) {
  const [settings, setSettings] = useState<AppSettings>({
    set_as_default_browser: false,
    theme: "system",
    custom_theme: undefined,
    api_enabled: false,
    api_port: 10108,
    api_token: undefined,
  });
  const [originalSettings, setOriginalSettings] = useState<AppSettings>({
    set_as_default_browser: false,
    theme: "system",
    custom_theme: undefined,
    api_enabled: false,
    api_port: 10108,
    api_token: undefined,
  });
  const [customThemeState, setCustomThemeState] = useState<CustomThemeState>({
    selectedThemeId: null,
    colors: {},
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
  const [apiServerPort, setApiServerPort] = useState<number | null>(null);

  const { setTheme } = useTheme();
  const {
    requestPermission,
    isMicrophoneAccessGranted,
    isCameraAccessGranted,
  } = usePermissions();
  const { termsAccepted } = useWayfernTerms();
  const { trialStatus } = useCommercialTrial();
  const [mcpEnabled, setMcpEnabled] = useState(false);
  const [isMcpStarting, setIsMcpStarting] = useState(false);

  const getPermissionIcon = useCallback((type: PermissionType) => {
    switch (type) {
      case "microphone":
        return <BsMic className="w-4 h-4" />;
      case "camera":
        return <BsCamera className="w-4 h-4" />;
    }
  }, []);

  const getPermissionDisplayName = useCallback((type: PermissionType) => {
    switch (type) {
      case "microphone":
        return "Microphone";
      case "camera":
        return "Camera";
    }
  }, []);

  const getStatusBadge = useCallback((isGranted: boolean) => {
    if (isGranted) {
      return (
        <Badge variant="default" className="text-green-800 bg-green-100">
          Granted
        </Badge>
      );
    }
    return <Badge variant="secondary">Not Granted</Badge>;
  }, []);

  const getPermissionDescription = useCallback((type: PermissionType) => {
    switch (type) {
      case "microphone":
        return "Access to microphone for browser applications";
      case "camera":
        return "Access to camera for browser applications";
    }
  }, []);

  const loadSettings = useCallback(async () => {
    setIsLoading(true);
    try {
      const appSettings = await invoke<AppSettings>("get_app_settings");
      const tokyoNightTheme = getThemeById("tokyo-night");
      if (!tokyoNightTheme) {
        throw new Error("Tokyo Night theme not found");
      }
      const merged: AppSettings = {
        ...appSettings,
        custom_theme:
          appSettings.custom_theme &&
          Object.keys(appSettings.custom_theme).length > 0
            ? appSettings.custom_theme
            : tokyoNightTheme.colors,
      };
      setSettings(merged);
      setOriginalSettings(merged);

      // Initialize custom theme state
      if (merged.theme === "custom" && merged.custom_theme) {
        const matchingTheme = getThemeByColors(merged.custom_theme);
        setCustomThemeState({
          selectedThemeId: matchingTheme?.id || null,
          colors: merged.custom_theme,
        });
      } else if (merged.theme === "custom") {
        // Initialize with Tokyo Night if no custom theme exists
        setCustomThemeState({
          selectedThemeId: "tokyo-night",
          colors: tokyoNightTheme.colors,
        });
      }
    } catch (error) {
      console.error("Failed to load settings:", error);
    } finally {
      setIsLoading(false);
    }
  }, []);

  const applyCustomTheme = useCallback((vars: Record<string, string>) => {
    const root = document.documentElement;
    Object.entries(vars).forEach(([k, v]) =>
      root.style.setProperty(k, v, "important"),
    );
  }, []);

  const clearCustomTheme = useCallback(() => {
    const root = document.documentElement;
    THEME_VARIABLES.forEach(({ key }) =>
      root.style.removeProperty(key as string),
    );
  }, []);

  const loadPermissions = useCallback(async () => {
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
  }, [
    getPermissionDescription,
    isCameraAccessGranted,
    isMacOS,
    isMicrophoneAccessGranted,
  ]);

  const checkDefaultBrowserStatus = useCallback(async () => {
    try {
      const isDefault = await invoke<boolean>("is_default_browser");
      setIsDefaultBrowser(isDefault);
    } catch (error) {
      console.error("Failed to check default browser status:", error);
    }
  }, []);

  const handleSetDefaultBrowser = useCallback(async () => {
    setIsSettingDefault(true);
    try {
      await invoke("set_as_default_browser");
      await checkDefaultBrowserStatus();
    } catch (error) {
      console.error("Failed to set as default browser:", error);
    } finally {
      setIsSettingDefault(false);
    }
  }, [checkDefaultBrowserStatus]);

  const handleClearCache = useCallback(async () => {
    setIsClearingCache(true);
    try {
      await invoke("clear_all_version_cache_and_refetch");
      // Also clear traffic stats cache
      await invoke("clear_all_traffic_stats");
      // Don't show immediate success toast - let the version update progress events handle it
    } catch (error) {
      console.error("Failed to clear cache:", error);
      showErrorToast("Failed to clear cache", {
        description:
          error instanceof Error ? error.message : "Unknown error occurred",
        duration: 4000,
      });
    } finally {
      setIsClearingCache(false);
    }
  }, []);

  const handleRequestPermission = useCallback(
    async (permissionType: PermissionType) => {
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
    },
    [getPermissionDisplayName, requestPermission],
  );

  const handleSave = useCallback(async () => {
    setIsSaving(true);
    try {
      // Update settings with current custom theme state
      let settingsToSave: AppSettings = {
        ...settings,
        custom_theme:
          settings.theme === "custom"
            ? customThemeState.colors
            : settings.custom_theme,
      };

      const savedSettings = await invoke<AppSettings>("save_app_settings", {
        settings: settingsToSave,
      });
      // Update settings with any generated tokens
      setSettings(savedSettings);
      settingsToSave = savedSettings;
      setTheme(settings.theme === "custom" ? "dark" : settings.theme);

      // Apply or clear custom variables only on Save
      if (settings.theme === "custom") {
        if (
          customThemeState.colors &&
          Object.keys(customThemeState.colors).length > 0
        ) {
          try {
            const root = document.documentElement;
            // Clear any previous custom vars first
            THEME_VARIABLES.forEach(({ key }) =>
              root.style.removeProperty(key as string),
            );
            Object.entries(customThemeState.colors).forEach(([k, v]) =>
              root.style.setProperty(k, v, "important"),
            );
          } catch {}
        }
      } else {
        try {
          const root = document.documentElement;
          THEME_VARIABLES.forEach(({ key }) =>
            root.style.removeProperty(key as string),
          );
        } catch {}
      }

      // Handle API server start/stop based on settings
      const wasApiEnabled = originalSettings.api_enabled;
      const isApiEnabled = settingsToSave.api_enabled;

      if (isApiEnabled && !wasApiEnabled) {
        // Start API server
        try {
          const port = await invoke<number>("start_api_server", {
            port: settingsToSave.api_port,
          });
          setApiServerPort(port);
          showSuccessToast(`Local API started on port ${port}`);
        } catch (error) {
          console.error("Failed to start API server:", error);
          showErrorToast("Failed to start API server", {
            description:
              error instanceof Error ? error.message : "Unknown error occurred",
          });
          // Revert the API enabled setting if start failed
          settingsToSave.api_enabled = false;
          const revertedSettings = await invoke<AppSettings>(
            "save_app_settings",
            { settings: settingsToSave },
          );
          setSettings(revertedSettings);
          settingsToSave = revertedSettings;
        }
      } else if (!isApiEnabled && wasApiEnabled) {
        // Stop API server
        try {
          await invoke("stop_api_server");
          setApiServerPort(null);
          showSuccessToast("Local API stopped");
        } catch (error) {
          console.error("Failed to stop API server:", error);
          showErrorToast("Failed to stop API server", {
            description:
              error instanceof Error ? error.message : "Unknown error occurred",
          });
        }
      }

      setOriginalSettings(settingsToSave);
      onClose();
    } catch (error) {
      console.error("Failed to save settings:", error);
    } finally {
      setIsSaving(false);
    }
  }, [onClose, setTheme, settings, customThemeState, originalSettings]);

  const updateSetting = useCallback(
    (
      key: keyof AppSettings,
      value: boolean | string | Record<string, string> | undefined,
    ) => {
      setSettings((prev) => ({ ...prev, [key]: value as unknown as never }));
    },
    [],
  );

  const loadApiServerStatus = useCallback(async () => {
    try {
      const port = await invoke<number | null>("get_api_server_status");
      setApiServerPort(port);
    } catch (error) {
      console.error("Failed to load API server status:", error);
      setApiServerPort(null);
    }
  }, []);

  const loadMcpServerStatus = useCallback(async () => {
    try {
      const isRunning = await invoke<boolean>("get_mcp_server_status");
      setMcpEnabled(isRunning);
    } catch (error) {
      console.error("Failed to load MCP server status:", error);
      setMcpEnabled(false);
    }
  }, []);

  const handleClose = useCallback(() => {
    // Restore original theme when closing without saving
    if (originalSettings.theme === "custom" && originalSettings.custom_theme) {
      applyCustomTheme(originalSettings.custom_theme);
    } else {
      clearCustomTheme();
    }

    // Reset custom theme state to original
    if (originalSettings.theme === "custom" && originalSettings.custom_theme) {
      const matchingTheme = getThemeByColors(originalSettings.custom_theme);
      setCustomThemeState({
        selectedThemeId: matchingTheme?.id || null,
        colors: originalSettings.custom_theme,
      });
    }

    onClose();
  }, [
    originalSettings.theme,
    originalSettings.custom_theme,
    applyCustomTheme,
    clearCustomTheme,
    onClose,
  ]);

  // Only clear custom theme when switching away from custom, don't apply live changes
  useEffect(() => {
    if (settings.theme !== "custom") {
      clearCustomTheme();
    }
  }, [settings.theme, clearCustomTheme]);

  useEffect(() => {
    if (isOpen) {
      loadSettings().catch(console.error);
      checkDefaultBrowserStatus().catch(console.error);
      loadApiServerStatus().catch(console.error);
      loadMcpServerStatus().catch(console.error);

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
  }, [
    isOpen,
    loadPermissions,
    checkDefaultBrowserStatus,
    loadSettings,
    loadApiServerStatus,
    loadMcpServerStatus,
  ]);

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

  // Check if settings have changed (excluding default browser setting)
  const hasChanges =
    settings.theme !== originalSettings.theme ||
    settings.api_enabled !== originalSettings.api_enabled ||
    (settings.theme === "custom" &&
      JSON.stringify(customThemeState.colors) !==
        JSON.stringify(originalSettings.custom_theme ?? {})) ||
    (settings.theme !== "custom" &&
      JSON.stringify(settings.custom_theme ?? {}) !==
        JSON.stringify(originalSettings.custom_theme ?? {}));

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md max-h-[80vh] my-8 flex flex-col">
        <DialogHeader className="shrink-0">
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
                  if (value === "custom") {
                    const tokyoNightTheme = getThemeById("tokyo-night");
                    if (tokyoNightTheme) {
                      setCustomThemeState({
                        selectedThemeId: "tokyo-night",
                        colors: tokyoNightTheme.colors,
                      });
                    }
                  }
                }}
              >
                <SelectTrigger id="theme-select">
                  <SelectValue placeholder="Select theme" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="light">Light</SelectItem>
                  <SelectItem value="dark">Dark</SelectItem>
                  <SelectItem value="system">System</SelectItem>
                  <SelectItem value="custom">Custom</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <p className="text-xs text-muted-foreground">
              Choose your preferred theme or follow your system settings. Custom
              theme changes are applied only when you save.
            </p>

            {settings.theme === "custom" && (
              <div className="space-y-3">
                <div className="space-y-2">
                  <Label
                    htmlFor="theme-preset-select"
                    className="text-sm font-medium"
                  >
                    Theme Preset
                  </Label>
                  <Select
                    value={customThemeState.selectedThemeId || "custom"}
                    onValueChange={(value) => {
                      if (value === "custom") {
                        setCustomThemeState((prev) => ({
                          ...prev,
                          selectedThemeId: null,
                        }));
                      } else {
                        const theme = getThemeById(value);
                        if (theme) {
                          setCustomThemeState({
                            selectedThemeId: value,
                            colors: theme.colors,
                          });
                        }
                      }
                    }}
                  >
                    <SelectTrigger id="theme-preset-select">
                      <SelectValue placeholder="Select a theme preset" />
                    </SelectTrigger>
                    <SelectContent>
                      {THEMES.map((theme) => (
                        <SelectItem key={theme.id} value={theme.id}>
                          {theme.name}
                        </SelectItem>
                      ))}
                      <SelectItem value="custom">Your Own</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <div className="text-sm font-medium">Custom Colors</div>
                <div className="grid grid-cols-4 gap-3">
                  {THEME_VARIABLES.map(({ key, label }) => {
                    const colorValue =
                      customThemeState.colors[key] || "#000000";
                    return (
                      <div
                        key={key}
                        className="flex flex-col gap-1 items-center"
                      >
                        <Popover>
                          <PopoverTrigger asChild>
                            <button
                              type="button"
                              aria-label={label}
                              className="w-8 h-8 rounded-md border shadow-sm cursor-pointer"
                              style={{ backgroundColor: colorValue }}
                            />
                          </PopoverTrigger>
                          <PopoverContent
                            className="w-[320px] p-3"
                            sideOffset={6}
                          >
                            <ColorPicker
                              className="p-3 rounded-md border shadow-sm bg-background"
                              value={colorValue}
                              onColorChange={([r, g, b, a]) => {
                                const next = Color({ r, g, b }).alpha(a);
                                const nextStr = next.hexa();
                                const newColors = {
                                  ...customThemeState.colors,
                                  [key]: nextStr,
                                };

                                // Check if colors match any preset theme
                                const matchingTheme =
                                  getThemeByColors(newColors);

                                setCustomThemeState({
                                  selectedThemeId: matchingTheme?.id || null,
                                  colors: newColors,
                                });
                              }}
                            >
                              <ColorPickerSelection className="h-36 rounded" />
                              <div className="flex gap-3 items-center mt-3">
                                <ColorPickerEyeDropper />
                                <div className="grid gap-1 w-full">
                                  <ColorPickerHue />
                                  <ColorPickerAlpha />
                                </div>
                              </div>
                              <div className="flex gap-2 items-center mt-3">
                                <ColorPickerOutput />
                                <ColorPickerFormat />
                              </div>
                            </ColorPicker>
                          </PopoverContent>
                        </Popover>
                        <div className="text-[10px] text-muted-foreground text-center leading-tight">
                          {label}
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}
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
                      className="flex justify-between items-center p-3 rounded-lg border"
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

          {/* Local API Section */}
          <div className="space-y-4">
            <Label className="text-base font-medium">Local API</Label>

            <div className="flex items-center space-x-2">
              <Checkbox
                id="api-enabled"
                checked={settings.api_enabled}
                onCheckedChange={async (checked: boolean) => {
                  updateSetting("api_enabled", checked);
                  try {
                    if (checked) {
                      // Ask backend to enable API and return settings with token
                      const next = await invoke<AppSettings>(
                        "save_app_settings",
                        {
                          settings: { ...settings, api_enabled: true },
                        },
                      );
                      setSettings(next);
                    } else {
                      const next = await invoke<AppSettings>(
                        "save_app_settings",
                        {
                          settings: {
                            ...settings,
                            api_enabled: false,
                            api_token: null,
                          },
                        },
                      );
                      setSettings(next);
                    }
                  } catch (e) {
                    console.error("Failed to toggle API:", e);
                  }
                }}
              />
              <div className="grid gap-1.5 leading-none">
                <Label
                  htmlFor="api-enabled"
                  className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                >
                  (ALPHA) Enable Local API Server
                </Label>
                <p className="text-xs text-muted-foreground">
                  Allow managing the application data externally via REST API.
                  Server will start on port 10108 or a random port if
                  unavailable.
                  {apiServerPort && (
                    <span className="ml-1 font-medium text-green-600">
                      (Currently running on port {apiServerPort})
                    </span>
                  )}
                </p>
              </div>
            </div>

            {settings.api_enabled && settings.api_token && (
              <div className="space-y-2">
                <Label className="text-sm font-medium">
                  API Authentication Token
                </Label>
                <div className="flex items-center space-x-2">
                  <input
                    type="text"
                    value={settings.api_token}
                    readOnly
                    className="flex-1 px-3 py-2 font-mono text-sm rounded-md border bg-muted"
                  />
                  <CopyToClipboard
                    text={settings.api_token || ""}
                    successMessage="API token copied to clipboard"
                  />
                </div>
                <p className="text-xs text-muted-foreground">
                  Include this token in the Authorization header as "Bearer{" "}
                  {settings.api_token}" for all API requests.
                </p>
                {/* Temporary in-app API docs */}
                <div className="p-3 mt-3 space-y-2 text-xs leading-relaxed rounded-md border bg-muted/40">
                  <div className="font-medium">
                    Temporary in-app API docs (alpha)
                  </div>
                  <div>
                    <div>
                      Base URL:{" "}
                      <code className="font-mono">{`http://127.0.0.1:${apiServerPort ?? settings.api_port ?? 10108}/v1`}</code>
                    </div>
                    <div>
                      Auth:{" "}
                      <code className="font-mono">
                        Authorization: Bearer {settings.api_token}
                      </code>
                    </div>
                  </div>
                  <div className="space-y-1">
                    <div className="font-medium">Profiles</div>
                    <ul className="list-disc ml-5 space-y-0.5">
                      <li>
                        <code className="font-mono">GET /profiles</code> — list
                        profiles
                      </li>
                      <li>
                        <code className="font-mono">
                          GET /profiles/{"{"}id{"}"}
                        </code>{" "}
                        — get one
                      </li>
                      <li>
                        <code className="font-mono">POST /profiles</code> —
                        create
                        <span className="ml-1 text-muted-foreground">
                          (required: name, browser, version; optional:
                          release_type, proxy_id, camoufox_config, group_id,
                          tags)
                        </span>
                      </li>
                      <li>
                        <code className="font-mono">
                          PUT /profiles/{"{"}id{"}"}
                        </code>{" "}
                        — update
                        <span className="ml-1 text-muted-foreground">
                          (any of: name, version, proxy_id, camoufox_config,
                          group_id, tags)
                        </span>
                      </li>
                      <li>
                        <code className="font-mono">
                          DELETE /profiles/{"{"}id{"}"}
                        </code>{" "}
                        — delete
                      </li>
                      <li>
                        <code className="font-mono">
                          POST /profiles/{"{"}id{"}"}/run
                        </code>{" "}
                        — launch with remote debugging
                        <span className="ml-1 text-muted-foreground">
                          (body: {"{"}url?, headless?{"}"})
                        </span>
                      </li>
                      <li>
                        <code className="font-mono">
                          POST /profiles/{"{"}id{"}"}/open-url
                        </code>{" "}
                        — open URL in running profile
                        <span className="ml-1 text-muted-foreground">
                          (body: {"{"}url{"}"})
                        </span>
                      </li>
                      <li>
                        <code className="font-mono">
                          POST /profiles/{"{"}id{"}"}/kill
                        </code>{" "}
                        — stop browser process
                      </li>
                    </ul>
                  </div>
                  <div className="space-y-1">
                    <div className="font-medium">Groups</div>
                    <ul className="list-disc ml-5 space-y-0.5">
                      <li>
                        <code className="font-mono">GET /groups</code> — list
                      </li>
                      <li>
                        <code className="font-mono">
                          GET /groups/{"{"}id{"}"}
                        </code>{" "}
                        — get one
                      </li>
                      <li>
                        <code className="font-mono">POST /groups</code> — create
                        <span className="ml-1 text-muted-foreground">
                          (required: name)
                        </span>
                      </li>
                      <li>
                        <code className="font-mono">
                          PUT /groups/{"{"}id{"}"}
                        </code>{" "}
                        — rename
                        <span className="ml-1 text-muted-foreground">
                          (required: name)
                        </span>
                      </li>
                      <li>
                        <code className="font-mono">
                          DELETE /groups/{"{"}id{"}"}
                        </code>{" "}
                        — delete
                      </li>
                    </ul>
                  </div>
                  <div className="space-y-1">
                    <div className="font-medium">Tags</div>
                    <ul className="list-disc ml-5 space-y-0.5">
                      <li>
                        <code className="font-mono">GET /tags</code> — list
                      </li>
                    </ul>
                  </div>
                  <div className="space-y-1">
                    <div className="font-medium">Proxies</div>
                    <ul className="list-disc ml-5 space-y-0.5">
                      <li>
                        <code className="font-mono">GET /proxies</code> — list
                      </li>
                      <li>
                        <code className="font-mono">
                          GET /proxies/{"{"}id{"}"}
                        </code>{" "}
                        — get one
                      </li>
                      <li>
                        <code className="font-mono">POST /proxies</code> —
                        create
                        <span className="ml-1 text-muted-foreground">
                          (required: name, proxy_settings object)
                        </span>
                      </li>
                      <li>
                        <code className="font-mono">
                          PUT /proxies/{"{"}id{"}"}
                        </code>{" "}
                        — update
                        <span className="ml-1 text-muted-foreground">
                          (optional: name, proxy_settings)
                        </span>
                      </li>
                      <li>
                        <code className="font-mono">
                          DELETE /proxies/{"{"}id{"}"}
                        </code>{" "}
                        — delete
                      </li>
                    </ul>
                  </div>
                  <div className="space-y-1">
                    <div className="font-medium">Browsers</div>
                    <ul className="list-disc ml-5 space-y-0.5">
                      <li>
                        <code className="font-mono">
                          POST /browsers/download
                        </code>{" "}
                        — download
                        <span className="ml-1 text-muted-foreground">
                          (required: browser, version)
                        </span>
                      </li>
                      <li>
                        <code className="font-mono">
                          GET /browsers/{"{"}browser{"}"}/versions
                        </code>{" "}
                        — list versions
                      </li>
                      <li>
                        <code className="font-mono">
                          GET /browsers/{"{"}browser{"}"}/versions/{"{"}version
                          {"}"}/downloaded
                        </code>{" "}
                        — is downloaded
                      </li>
                    </ul>
                  </div>
                  <div className="text-muted-foreground">
                    These docs are temporary and will be replaced with full
                    documentation later.
                  </div>
                </div>
              </div>
            )}
          </div>

          {/* Commercial License Section */}
          <div className="space-y-4">
            <Label className="text-base font-medium">Commercial License</Label>

            <div className="flex items-center justify-between p-3 rounded-md border bg-muted/40">
              {trialStatus?.type === "Active" ? (
                <div className="space-y-1">
                  <p className="text-sm font-medium">
                    Trial: {trialStatus.days_remaining} days,{" "}
                    {trialStatus.hours_remaining} hours remaining
                  </p>
                  <p className="text-xs text-muted-foreground">
                    Commercial use is free during the trial period
                  </p>
                </div>
              ) : (
                <div className="space-y-1">
                  <p className="text-sm font-medium text-orange-600">
                    Trial expired
                  </p>
                  <p className="text-xs text-muted-foreground">
                    Personal use remains free. Commercial use requires a
                    license.
                  </p>
                </div>
              )}
            </div>
          </div>

          {/* MCP Server Section */}
          <div className="space-y-4">
            <Label className="text-base font-medium">MCP Server</Label>

            <div className="flex items-center space-x-2">
              <Checkbox
                id="mcp-enabled"
                checked={mcpEnabled}
                disabled={!termsAccepted || isMcpStarting}
                onCheckedChange={async (checked: boolean) => {
                  setIsMcpStarting(true);
                  try {
                    if (checked) {
                      await invoke("start_mcp_server");
                      setMcpEnabled(true);
                      showSuccessToast("MCP server started");
                    } else {
                      await invoke("stop_mcp_server");
                      setMcpEnabled(false);
                      showSuccessToast("MCP server stopped");
                    }
                  } catch (e) {
                    console.error("Failed to toggle MCP server:", e);
                    showErrorToast("Failed to toggle MCP server", {
                      description:
                        e instanceof Error ? e.message : "Unknown error",
                    });
                  } finally {
                    setIsMcpStarting(false);
                  }
                }}
              />
              <div className="grid gap-1.5 leading-none">
                <Label
                  htmlFor="mcp-enabled"
                  className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                >
                  Enable MCP Server (Model Context Protocol)
                </Label>
                <p className="text-xs text-muted-foreground">
                  Allow AI assistants to control Wayfern and Camoufox browsers
                  via MCP.
                  {!termsAccepted && (
                    <span className="ml-1 text-orange-600">
                      (Accept terms first)
                    </span>
                  )}
                </p>
              </div>
            </div>

            {mcpEnabled && (
              <div className="p-3 space-y-2 text-xs rounded-md border bg-muted/40">
                <div className="font-medium">Available MCP Tools</div>
                <ul className="list-disc ml-5 space-y-0.5 text-muted-foreground">
                  <li>list_profiles - List Wayfern/Camoufox profiles</li>
                  <li>run_profile - Launch a browser profile</li>
                  <li>kill_profile - Stop a running browser</li>
                  <li>get_profile - Get profile details</li>
                  <li>list_proxies - List configured proxies</li>
                </ul>
              </div>
            )}
          </div>

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

        <DialogFooter className="shrink-0">
          <RippleButton variant="outline" onClick={handleClose}>
            Cancel
          </RippleButton>
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
