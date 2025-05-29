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
  });
  const [originalSettings, setOriginalSettings] = useState<AppSettings>({
    set_as_default_browser: false,
    show_settings_on_startup: true,
    theme: "system",
    auto_updates_enabled: true,
  });
  const [isDefaultBrowser, setIsDefaultBrowser] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [isSettingDefault, setIsSettingDefault] = useState(false);

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
    settings.auto_updates_enabled !== originalSettings.auto_updates_enabled;

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md max-h-[80vh] my-8 flex flex-col">
        <DialogHeader className="flex-shrink-0">
          <DialogTitle>Settings</DialogTitle>
        </DialogHeader>

        <div className="grid gap-6 py-4 overflow-y-auto flex-1 min-h-0">
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
            <div className="flex items-center justify-between">
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

            <p className="text-xs text-muted-foreground">
              When enabled, Donut Browser will check for browser updates and
              notify you when updates are available for your profiles.
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
