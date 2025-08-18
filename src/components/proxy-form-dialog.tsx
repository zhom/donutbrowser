"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
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
import type { StoredProxy } from "@/types";
import { RippleButton } from "./ui/ripple";

interface ProxyFormData {
  name: string;
  proxy_type: string;
  host: string;
  port: number;
  username: string;
  password: string;
}

interface ProxyFormDialogProps {
  isOpen: boolean;
  onClose: () => void;
  editingProxy?: StoredProxy | null;
}

export function ProxyFormDialog({
  isOpen,
  onClose,
  editingProxy,
}: ProxyFormDialogProps) {
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [formData, setFormData] = useState<ProxyFormData>({
    name: "",
    proxy_type: "http",
    host: "",
    port: 8080,
    username: "",
    password: "",
  });

  const resetForm = useCallback(() => {
    setFormData({
      name: "",
      proxy_type: "http",
      host: "",
      port: 8080,
      username: "",
      password: "",
    });
  }, []);

  // Load editing proxy data when dialog opens
  useEffect(() => {
    if (isOpen) {
      if (editingProxy) {
        setFormData({
          name: editingProxy.name,
          proxy_type: editingProxy.proxy_settings.proxy_type,
          host: editingProxy.proxy_settings.host,
          port: editingProxy.proxy_settings.port,
          username: editingProxy.proxy_settings.username || "",
          password: editingProxy.proxy_settings.password || "",
        });
      } else {
        resetForm();
      }
    }
  }, [isOpen, editingProxy, resetForm]);

  const handleSubmit = useCallback(async () => {
    if (!formData.name.trim()) {
      toast.error("Proxy name is required");
      return;
    }

    if (!formData.host.trim() || !formData.port) {
      toast.error("Host and port are required");
      return;
    }

    setIsSubmitting(true);
    try {
      const proxySettings = {
        proxy_type: formData.proxy_type,
        host: formData.host.trim(),
        port: formData.port,
        username: formData.username.trim() || undefined,
        password: formData.password.trim() || undefined,
      };

      if (editingProxy) {
        // Update existing proxy
        await invoke("update_stored_proxy", {
          proxyId: editingProxy.id,
          name: formData.name.trim(),
          proxySettings,
        });
        toast.success("Proxy updated successfully");
      } else {
        // Create new proxy
        await invoke("create_stored_proxy", {
          name: formData.name.trim(),
          proxySettings,
        });
        toast.success("Proxy created successfully");
      }

      onClose();
    } catch (error) {
      console.error("Failed to save proxy:", error);
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      toast.error(`Failed to save proxy: ${errorMessage}`);
    } finally {
      setIsSubmitting(false);
    }
  }, [formData, editingProxy, onClose]);

  const handleClose = useCallback(() => {
    if (!isSubmitting) {
      onClose();
    }
  }, [isSubmitting, onClose]);

  const isFormValid =
    formData.name.trim() &&
    formData.host.trim() &&
    formData.port > 0 &&
    formData.port <= 65535;

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            {editingProxy ? "Edit Proxy" : "Create New Proxy"}
          </DialogTitle>
        </DialogHeader>

        <div className="grid gap-4 py-4">
          <div className="grid gap-2">
            <Label htmlFor="proxy-name">Proxy Name</Label>
            <Input
              id="proxy-name"
              value={formData.name}
              onChange={(e) =>
                setFormData({ ...formData, name: e.target.value })
              }
              placeholder="e.g. Office Proxy, Home VPN, etc."
              disabled={isSubmitting}
            />
          </div>

          <div className="grid gap-2">
            <Label>Proxy Type</Label>
            <Select
              value={formData.proxy_type}
              onValueChange={(value) =>
                setFormData({ ...formData, proxy_type: value })
              }
              disabled={isSubmitting}
            >
              <SelectTrigger>
                <SelectValue placeholder="Select proxy type" />
              </SelectTrigger>
              <SelectContent>
                {["http", "https", "socks4", "socks5"].map((type) => (
                  <SelectItem key={type} value={type}>
                    {type.toUpperCase()}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div className="grid gap-2">
              <Label htmlFor="proxy-host">Host</Label>
              <Input
                id="proxy-host"
                value={formData.host}
                onChange={(e) =>
                  setFormData({ ...formData, host: e.target.value })
                }
                placeholder="e.g. 127.0.0.1"
                disabled={isSubmitting}
              />
            </div>

            <div className="grid gap-2">
              <Label htmlFor="proxy-port">Port</Label>
              <Input
                id="proxy-port"
                type="number"
                value={formData.port}
                onChange={(e) =>
                  setFormData({
                    ...formData,
                    port: parseInt(e.target.value, 10) || 0,
                  })
                }
                placeholder="e.g. 8080"
                min="1"
                max="65535"
                disabled={isSubmitting}
              />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div className="grid gap-2">
              <Label htmlFor="proxy-username">Username (optional)</Label>
              <Input
                id="proxy-username"
                value={formData.username}
                onChange={(e) =>
                  setFormData({
                    ...formData,
                    username: e.target.value,
                  })
                }
                placeholder="Proxy username"
                disabled={isSubmitting}
              />
            </div>

            <div className="grid gap-2">
              <Label htmlFor="proxy-password">Password (optional)</Label>
              <Input
                id="proxy-password"
                type="password"
                value={formData.password}
                onChange={(e) =>
                  setFormData({
                    ...formData,
                    password: e.target.value,
                  })
                }
                placeholder="Proxy password"
                disabled={isSubmitting}
              />
            </div>
          </div>
        </div>

        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={handleClose}
            disabled={isSubmitting}
          >
            Cancel
          </RippleButton>
          <LoadingButton
            isLoading={isSubmitting}
            onClick={handleSubmit}
            disabled={!isFormValid}
          >
            {editingProxy ? "Update Proxy" : "Create Proxy"}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
