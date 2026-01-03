"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { LuEye, LuEyeOff } from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { SyncSettings } from "@/types";

interface SyncConfigDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function SyncConfigDialog({ isOpen, onClose }: SyncConfigDialogProps) {
  const [serverUrl, setServerUrl] = useState("");
  const [token, setToken] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [isTesting, setIsTesting] = useState(false);
  const [showToken, setShowToken] = useState(false);

  const loadSettings = useCallback(async () => {
    setIsLoading(true);
    try {
      const settings = await invoke<SyncSettings>("get_sync_settings");
      setServerUrl(settings.sync_server_url || "");
      setToken(settings.sync_token || "");
    } catch (error) {
      console.error("Failed to load sync settings:", error);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      void loadSettings();
    }
  }, [isOpen, loadSettings]);

  const handleTestConnection = useCallback(async () => {
    if (!serverUrl) {
      showErrorToast("Please enter a server URL");
      return;
    }

    setIsTesting(true);
    try {
      const healthUrl = `${serverUrl.replace(/\/$/, "")}/health`;
      const response = await fetch(healthUrl);
      if (response.ok) {
        showSuccessToast("Connection successful!");
      } else {
        showErrorToast("Server responded with an error");
      }
    } catch {
      showErrorToast("Failed to connect to server");
    } finally {
      setIsTesting(false);
    }
  }, [serverUrl]);

  const handleSave = useCallback(async () => {
    setIsSaving(true);
    try {
      await invoke<SyncSettings>("save_sync_settings", {
        syncServerUrl: serverUrl || null,
        syncToken: token || null,
      });
      showSuccessToast("Sync settings saved");
      onClose();
    } catch (error) {
      console.error("Failed to save sync settings:", error);
      showErrorToast("Failed to save settings");
    } finally {
      setIsSaving(false);
    }
  }, [serverUrl, token, onClose]);

  const handleDisconnect = useCallback(async () => {
    setIsSaving(true);
    try {
      await invoke<SyncSettings>("save_sync_settings", {
        syncServerUrl: null,
        syncToken: null,
      });
      setServerUrl("");
      setToken("");
      showSuccessToast("Sync disconnected");
    } catch (error) {
      console.error("Failed to disconnect:", error);
      showErrorToast("Failed to disconnect");
    } finally {
      setIsSaving(false);
    }
  }, []);

  const isConnected = Boolean(serverUrl && token);

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Sync Service</DialogTitle>
          <DialogDescription>
            Configure connection to a sync server to synchronize your profiles
            across devices.
          </DialogDescription>
        </DialogHeader>

        {isLoading ? (
          <div className="flex justify-center py-8">
            <div className="w-6 h-6 rounded-full border-2 border-current animate-spin border-t-transparent" />
          </div>
        ) : (
          <div className="grid gap-4 py-4">
            <div className="space-y-2">
              <Label htmlFor="sync-server-url">Server URL</Label>
              <Input
                id="sync-server-url"
                placeholder="https://sync.example.com"
                value={serverUrl}
                onChange={(e) => setServerUrl(e.target.value)}
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="sync-token">Access Token</Label>
              <div className="relative">
                <Input
                  id="sync-token"
                  type={showToken ? "text" : "password"}
                  placeholder="Enter your sync token"
                  value={token}
                  onChange={(e) => setToken(e.target.value)}
                  className="pr-10"
                />
                <Tooltip>
                  <TooltipTrigger asChild>
                    <button
                      type="button"
                      onClick={() => setShowToken(!showToken)}
                      className="absolute right-3 top-1/2 p-1 rounded-sm transition-colors transform -translate-y-1/2 hover:bg-accent"
                      aria-label={showToken ? "Hide token" : "Show token"}
                    >
                      {showToken ? (
                        <LuEyeOff className="w-4 h-4 text-muted-foreground hover:text-foreground" />
                      ) : (
                        <LuEye className="w-4 h-4 text-muted-foreground hover:text-foreground" />
                      )}
                    </button>
                  </TooltipTrigger>
                  <TooltipContent>
                    {showToken ? "Hide token" : "Show token"}
                  </TooltipContent>
                </Tooltip>
              </div>
            </div>

            {isConnected && (
              <div className="flex gap-2 items-center text-sm text-muted-foreground">
                <div className="w-2 h-2 rounded-full bg-green-500" />
                Connected
              </div>
            )}
          </div>
        )}

        <DialogFooter className="flex gap-2">
          {isConnected && (
            <Button
              variant="outline"
              onClick={handleDisconnect}
              disabled={isSaving}
            >
              Disconnect
            </Button>
          )}
          <Button
            variant="outline"
            onClick={handleTestConnection}
            disabled={isTesting || !serverUrl}
          >
            {isTesting ? "Testing..." : "Test Connection"}
          </Button>
          <LoadingButton
            onClick={handleSave}
            isLoading={isSaving}
            disabled={!serverUrl || !token}
          >
            Save
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
