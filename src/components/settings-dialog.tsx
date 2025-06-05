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
import { invoke } from "@tauri-apps/api/core";
import { useTheme } from "next-themes";
import { useEffect, useState } from "react";

interface AppSettings {
  set_as_default_browser: boolean;
  show_settings_on_startup: boolean;
  theme: string;
  auto_updates_enabled: boolean;
  auto_delete_unused_binaries: boolean;
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
  const [isCleaningBinaries, setIsCleaningBinaries] = useState(false);

  const { setTheme } = useTheme();

  useEffect(() => {
    if (isOpen) {
      void loadSettings();
      void checkDefaultBrowserStatus();

      // Set up interval to check default browser status
      const intervalId = setInterval(() => {
        void checkDefaultBrowserStatus();
      }, 500); // Check every 2 seconds

      // Cleanup interval on component unmount or dialog close
      return () => {
        clearInterval(intervalId);
      };
    }
  }, [isOpen]);

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
      await invoke("clear_all_version_cache");
      // Optionally show a success message
      console.log("Cache cleared successfully");
    } catch (error) {
      console.error("Failed to clear cache:", error);
    } finally {
      setIsClearingCache(false);
    }
  };

  const handleCleanupBinaries = async () => {
    setIsCleaningBinaries(true);
    try {
      const cleanedUp = await invoke<string[]>("cleanup_unused_binaries");
      if (cleanedUp.length > 0) {
        console.log(
          `Cleaned up ${cleanedUp.length} unused binaries:`,
          cleanedUp,
        );
        // You could show a toast with the results
      } else {
        console.log("No unused binaries to clean up");
      }
    } catch (error) {
      console.error("Failed to cleanup unused binaries:", error);
    } finally {
      setIsCleaningBinaries(false);
    }
  };

  const handleSave = async () => {
    setIsSaving(true);
    try {
      await invoke("save_app_settings", { settings });
      // Apply theme change immediately
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
                void handleSetDefaultBrowser();
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

          {/* Advanced Section */}
          <div className="space-y-4">
            <Label className="text-base font-medium">Advanced</Label>

            <LoadingButton
              isLoading={isClearingCache}
              onClick={() => {
                void handleClearCache();
              }}
              variant="outline"
              className="w-full"
            >
              Clear All Version Cache
            </LoadingButton>

            <p className="text-xs text-muted-foreground">
              Clear all cached browser version data. This will force a fresh
              download of version information on the next app restart or manual
              refresh.
            </p>

            <LoadingButton
              isLoading={isCleaningBinaries}
              onClick={() => {
                void handleCleanupBinaries();
              }}
              variant="outline"
              className="w-full"
            >
              Clean Up Unused Binaries
            </LoadingButton>

            <p className="text-xs text-muted-foreground">
              Manually remove browser binaries that are not used by any profile.
              This can help free up disk space. Note: This will run
              automatically when the setting above is enabled.
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
              void handleSave();
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
