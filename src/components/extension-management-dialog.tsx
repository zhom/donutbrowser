"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import { LuPencil, LuPuzzle, LuTrash2, LuUpload } from "react-icons/lu";
import { Badge } from "@/components/ui/badge";
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
import { ProBadge } from "@/components/ui/pro-badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { Extension, ExtensionGroup } from "@/types";
import { DeleteConfirmationDialog } from "./delete-confirmation-dialog";
import { RippleButton } from "./ui/ripple";

interface ExtensionManagementDialogProps {
  isOpen: boolean;
  onClose: () => void;
  limitedMode: boolean;
}

export function ExtensionManagementDialog({
  isOpen,
  onClose,
  limitedMode,
}: ExtensionManagementDialogProps) {
  const { t } = useTranslation();
  const [extensions, setExtensions] = useState<Extension[]>([]);
  const [extensionGroups, setExtensionGroups] = useState<ExtensionGroup[]>([]);
  const [isLoading, setIsLoading] = useState(false);

  // Extension upload state
  const [isUploading, setIsUploading] = useState(false);
  const [extensionName, setExtensionName] = useState("");
  const [showUploadForm, setShowUploadForm] = useState(false);
  const [pendingFile, setPendingFile] = useState<{
    name: string;
    data: number[];
  } | null>(null);

  // Group state
  const [showCreateGroup, setShowCreateGroup] = useState(false);
  const [newGroupName, setNewGroupName] = useState("");
  const [editingGroup, setEditingGroup] = useState<ExtensionGroup | null>(null);
  const [editGroupName, setEditGroupName] = useState("");

  // Delete state
  const [extensionToDelete, setExtensionToDelete] = useState<Extension | null>(
    null,
  );
  const [groupToDelete, setGroupToDelete] = useState<ExtensionGroup | null>(
    null,
  );
  const [isDeleting, setIsDeleting] = useState(false);

  // Tab
  const [activeTab, setActiveTab] = useState<"extensions" | "groups">(
    "extensions",
  );

  const loadData = useCallback(async () => {
    if (limitedMode) return;
    setIsLoading(true);
    try {
      const [exts, groups] = await Promise.all([
        invoke<Extension[]>("list_extensions"),
        invoke<ExtensionGroup[]>("list_extension_groups"),
      ]);
      setExtensions(exts);
      setExtensionGroups(groups);
    } catch {
      // User may not have pro subscription
      setExtensions([]);
      setExtensionGroups([]);
    } finally {
      setIsLoading(false);
    }
  }, [limitedMode]);

  useEffect(() => {
    if (isOpen) {
      void loadData();
    }
  }, [isOpen, loadData]);

  const handleFileSelect = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;

      const validExtensions = [".xpi", ".crx", ".zip"];
      const isValid = validExtensions.some((ext) =>
        file.name.toLowerCase().endsWith(ext),
      );
      if (!isValid) {
        showErrorToast(t("extensions.invalidFileType"));
        return;
      }

      const reader = new FileReader();
      reader.onload = (event) => {
        const arrayBuffer = event.target?.result as ArrayBuffer;
        const data = Array.from(new Uint8Array(arrayBuffer));
        const baseName = file.name
          .replace(/\.(xpi|crx|zip)$/i, "")
          .replace(/[-_]/g, " ");
        setExtensionName(baseName);
        setPendingFile({ name: file.name, data });
        setShowUploadForm(true);
      };
      reader.onerror = () => {
        showErrorToast(t("extensions.readError"));
      };
      reader.readAsArrayBuffer(file);

      // Reset input
      e.target.value = "";
    },
    [t],
  );

  const handleUpload = useCallback(async () => {
    if (!pendingFile || !extensionName.trim()) return;
    setIsUploading(true);
    try {
      await invoke("add_extension", {
        name: extensionName.trim(),
        fileName: pendingFile.name,
        fileData: pendingFile.data,
      });
      showSuccessToast(t("extensions.uploadSuccess"));
      setShowUploadForm(false);
      setPendingFile(null);
      setExtensionName("");
      void loadData();
    } catch (err) {
      showErrorToast(err instanceof Error ? err.message : String(err));
    } finally {
      setIsUploading(false);
    }
  }, [pendingFile, extensionName, loadData, t]);

  const handleDeleteExtension = useCallback(async () => {
    if (!extensionToDelete) return;
    setIsDeleting(true);
    try {
      await invoke("delete_extension", { extensionId: extensionToDelete.id });
      showSuccessToast(t("extensions.deleteSuccess"));
      setExtensionToDelete(null);
      void loadData();
    } catch (err) {
      showErrorToast(err instanceof Error ? err.message : String(err));
    } finally {
      setIsDeleting(false);
    }
  }, [extensionToDelete, loadData, t]);

  const handleCreateGroup = useCallback(async () => {
    if (!newGroupName.trim()) return;
    try {
      await invoke("create_extension_group", { name: newGroupName.trim() });
      showSuccessToast(t("extensions.groupCreateSuccess"));
      setShowCreateGroup(false);
      setNewGroupName("");
      void loadData();
    } catch (err) {
      showErrorToast(err instanceof Error ? err.message : String(err));
    }
  }, [newGroupName, loadData, t]);

  const handleUpdateGroup = useCallback(async () => {
    if (!editingGroup || !editGroupName.trim()) return;
    try {
      await invoke("update_extension_group", {
        groupId: editingGroup.id,
        name: editGroupName.trim(),
      });
      showSuccessToast(t("extensions.groupUpdateSuccess"));
      setEditingGroup(null);
      setEditGroupName("");
      void loadData();
    } catch (err) {
      showErrorToast(err instanceof Error ? err.message : String(err));
    }
  }, [editingGroup, editGroupName, loadData, t]);

  const handleDeleteGroup = useCallback(async () => {
    if (!groupToDelete) return;
    setIsDeleting(true);
    try {
      await invoke("delete_extension_group", { groupId: groupToDelete.id });
      showSuccessToast(t("extensions.groupDeleteSuccess"));
      setGroupToDelete(null);
      void loadData();
    } catch (err) {
      showErrorToast(err instanceof Error ? err.message : String(err));
    } finally {
      setIsDeleting(false);
    }
  }, [groupToDelete, loadData, t]);

  const handleAddToGroup = useCallback(
    async (groupId: string, extensionId: string) => {
      try {
        await invoke("add_extension_to_group", { groupId, extensionId });
        void loadData();
      } catch (err) {
        showErrorToast(err instanceof Error ? err.message : String(err));
      }
    },
    [loadData],
  );

  const handleRemoveFromGroup = useCallback(
    async (groupId: string, extensionId: string) => {
      try {
        await invoke("remove_extension_from_group", { groupId, extensionId });
        void loadData();
      } catch (err) {
        showErrorToast(err instanceof Error ? err.message : String(err));
      }
    },
    [loadData],
  );

  const getCompatibilityBadge = (compat: string[]) => {
    if (compat.includes("chromium") && compat.includes("firefox")) {
      return (
        <Badge variant="secondary">{t("extensions.compatibility.both")}</Badge>
      );
    }
    if (compat.includes("chromium")) {
      return (
        <Badge variant="secondary">
          {t("extensions.compatibility.chromium")}
        </Badge>
      );
    }
    if (compat.includes("firefox")) {
      return (
        <Badge variant="secondary">
          {t("extensions.compatibility.firefox")}
        </Badge>
      );
    }
    return null;
  };

  return (
    <>
      <Dialog open={isOpen} onOpenChange={onClose}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <LuPuzzle className="w-5 h-5" />
              {t("extensions.title")}
              {limitedMode && <ProBadge />}
            </DialogTitle>
            <DialogDescription>{t("extensions.description")}</DialogDescription>
          </DialogHeader>

          <div className="relative">
            {limitedMode && (
              <>
                <div className="absolute inset-0 backdrop-blur-[6px] bg-background/30 z-[1]" />
                <div className="absolute inset-y-0 left-0 w-6 bg-gradient-to-r from-background to-transparent z-[2]" />
                <div className="absolute inset-y-0 right-0 w-6 bg-gradient-to-l from-background to-transparent z-[2]" />
                <div className="absolute inset-x-0 top-0 h-6 bg-gradient-to-b from-background to-transparent z-[2]" />
                <div className="absolute inset-x-0 bottom-0 h-6 bg-gradient-to-t from-background to-transparent z-[2]" />
                <div className="absolute inset-0 flex items-center justify-center z-[3]">
                  <div className="flex items-center gap-2 rounded-md bg-background/80 px-3 py-1.5">
                    <ProBadge />
                    <span className="text-sm font-medium text-muted-foreground">
                      {t("extensions.proRequired")}
                    </span>
                  </div>
                </div>
              </>
            )}

            <div className="space-y-4">
              {/* Tab selector */}
              <div className="flex gap-2 border-b">
                <button
                  type="button"
                  className={`px-3 py-2 text-sm font-medium border-b-2 transition-colors ${
                    activeTab === "extensions"
                      ? "border-primary text-foreground"
                      : "border-transparent text-muted-foreground hover:text-foreground"
                  }`}
                  onClick={() => setActiveTab("extensions")}
                  disabled={limitedMode}
                >
                  {t("extensions.extensionsTab")}
                </button>
                <button
                  type="button"
                  className={`px-3 py-2 text-sm font-medium border-b-2 transition-colors ${
                    activeTab === "groups"
                      ? "border-primary text-foreground"
                      : "border-transparent text-muted-foreground hover:text-foreground"
                  }`}
                  onClick={() => setActiveTab("groups")}
                  disabled={limitedMode}
                >
                  {t("extensions.groupsTab")}
                </button>
              </div>

              {/* Notice */}
              <div className="rounded-md bg-muted/50 p-3 text-sm text-muted-foreground">
                {t("extensions.managedNotice")}
              </div>

              {activeTab === "extensions" && (
                <div className="space-y-4">
                  <div className="flex justify-between items-center">
                    <Label>{t("extensions.extensionsTab")}</Label>
                    <div>
                      <label htmlFor="ext-file-input">
                        <RippleButton
                          size="sm"
                          className="flex gap-2 items-center"
                          disabled={limitedMode}
                          onClick={() =>
                            document.getElementById("ext-file-input")?.click()
                          }
                        >
                          <LuUpload className="w-4 h-4" />
                          {t("extensions.upload")}
                        </RippleButton>
                      </label>
                      <input
                        id="ext-file-input"
                        type="file"
                        accept=".xpi,.crx,.zip"
                        className="hidden"
                        onChange={handleFileSelect}
                        disabled={limitedMode}
                      />
                    </div>
                  </div>

                  {/* Upload form */}
                  {showUploadForm && pendingFile && (
                    <div className="space-y-3 rounded-md border p-3">
                      <div className="text-sm text-muted-foreground">
                        {t("extensions.selectedFile")}:{" "}
                        <span className="font-medium text-foreground">
                          {pendingFile.name}
                        </span>
                      </div>
                      <div className="flex gap-2">
                        <Input
                          value={extensionName}
                          onChange={(e) => setExtensionName(e.target.value)}
                          placeholder={t("extensions.namePlaceholder")}
                          className="flex-1"
                        />
                        <RippleButton
                          size="sm"
                          onClick={handleUpload}
                          disabled={isUploading || !extensionName.trim()}
                        >
                          {isUploading
                            ? t("common.buttons.loading")
                            : t("common.buttons.add")}
                        </RippleButton>
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => {
                            setShowUploadForm(false);
                            setPendingFile(null);
                            setExtensionName("");
                          }}
                        >
                          {t("common.buttons.cancel")}
                        </Button>
                      </div>
                    </div>
                  )}

                  {/* Extensions list */}
                  {isLoading ? (
                    <div className="text-sm text-muted-foreground">
                      {t("common.buttons.loading")}
                    </div>
                  ) : extensions.length === 0 ? (
                    <div className="text-sm text-muted-foreground">
                      {t("extensions.empty")}
                    </div>
                  ) : (
                    <div className="border rounded-md">
                      <ScrollArea className="h-[200px]">
                        <Table>
                          <TableHeader>
                            <TableRow>
                              <TableHead>{t("common.labels.name")}</TableHead>
                              <TableHead className="w-24">
                                {t("common.labels.type")}
                              </TableHead>
                              <TableHead className="w-32">
                                {t("extensions.compatibility.label")}
                              </TableHead>
                              <TableHead className="w-20">
                                {t("common.labels.actions")}
                              </TableHead>
                            </TableRow>
                          </TableHeader>
                          <TableBody>
                            {extensions.map((ext) => (
                              <TableRow key={ext.id}>
                                <TableCell className="font-medium">
                                  {ext.name}
                                </TableCell>
                                <TableCell>
                                  <Badge variant="outline">
                                    .{ext.file_type}
                                  </Badge>
                                </TableCell>
                                <TableCell>
                                  {getCompatibilityBadge(
                                    ext.browser_compatibility,
                                  )}
                                </TableCell>
                                <TableCell>
                                  <Tooltip>
                                    <TooltipTrigger asChild>
                                      <Button
                                        variant="ghost"
                                        size="sm"
                                        onClick={() =>
                                          setExtensionToDelete(ext)
                                        }
                                      >
                                        <LuTrash2 className="w-4 h-4" />
                                      </Button>
                                    </TooltipTrigger>
                                    <TooltipContent>
                                      {t("extensions.delete")}
                                    </TooltipContent>
                                  </Tooltip>
                                </TableCell>
                              </TableRow>
                            ))}
                          </TableBody>
                        </Table>
                      </ScrollArea>
                    </div>
                  )}
                </div>
              )}

              {activeTab === "groups" && (
                <div className="space-y-4">
                  <div className="flex justify-between items-center">
                    <Label>{t("extensions.groupsTab")}</Label>
                    <RippleButton
                      size="sm"
                      onClick={() => setShowCreateGroup(true)}
                      className="flex gap-2 items-center"
                      disabled={limitedMode}
                    >
                      <GoPlus className="w-4 h-4" />
                      {t("extensions.createGroup")}
                    </RippleButton>
                  </div>

                  {/* Create group form */}
                  {showCreateGroup && (
                    <div className="flex gap-2 items-center">
                      <Input
                        value={newGroupName}
                        onChange={(e) => setNewGroupName(e.target.value)}
                        placeholder={t("extensions.groupNamePlaceholder")}
                        className="flex-1"
                        onKeyDown={(e) => {
                          if (e.key === "Enter") void handleCreateGroup();
                        }}
                      />
                      <RippleButton
                        size="sm"
                        onClick={handleCreateGroup}
                        disabled={!newGroupName.trim()}
                      >
                        {t("common.buttons.create")}
                      </RippleButton>
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => {
                          setShowCreateGroup(false);
                          setNewGroupName("");
                        }}
                      >
                        {t("common.buttons.cancel")}
                      </Button>
                    </div>
                  )}

                  {/* Groups list */}
                  {extensionGroups.length === 0 ? (
                    <div className="text-sm text-muted-foreground">
                      {t("extensions.noGroups")}
                    </div>
                  ) : (
                    <div className="space-y-3">
                      {extensionGroups.map((group) => (
                        <div
                          key={group.id}
                          className="rounded-md border p-3 space-y-2"
                        >
                          <div className="flex justify-between items-center">
                            {editingGroup?.id === group.id ? (
                              <div className="flex gap-2 items-center flex-1">
                                <Input
                                  value={editGroupName}
                                  onChange={(e) =>
                                    setEditGroupName(e.target.value)
                                  }
                                  className="flex-1"
                                  onKeyDown={(e) => {
                                    if (e.key === "Enter")
                                      void handleUpdateGroup();
                                  }}
                                />
                                <RippleButton
                                  size="sm"
                                  onClick={handleUpdateGroup}
                                >
                                  {t("common.buttons.save")}
                                </RippleButton>
                                <Button
                                  size="sm"
                                  variant="outline"
                                  onClick={() => setEditingGroup(null)}
                                >
                                  {t("common.buttons.cancel")}
                                </Button>
                              </div>
                            ) : (
                              <>
                                <span className="font-medium">
                                  {group.name}
                                </span>
                                <div className="flex gap-1">
                                  <Tooltip>
                                    <TooltipTrigger asChild>
                                      <Button
                                        variant="ghost"
                                        size="sm"
                                        onClick={() => {
                                          setEditingGroup(group);
                                          setEditGroupName(group.name);
                                        }}
                                      >
                                        <LuPencil className="w-4 h-4" />
                                      </Button>
                                    </TooltipTrigger>
                                    <TooltipContent>
                                      {t("common.buttons.edit")}
                                    </TooltipContent>
                                  </Tooltip>
                                  <Tooltip>
                                    <TooltipTrigger asChild>
                                      <Button
                                        variant="ghost"
                                        size="sm"
                                        onClick={() => setGroupToDelete(group)}
                                      >
                                        <LuTrash2 className="w-4 h-4" />
                                      </Button>
                                    </TooltipTrigger>
                                    <TooltipContent>
                                      {t("extensions.deleteGroup")}
                                    </TooltipContent>
                                  </Tooltip>
                                </div>
                              </>
                            )}
                          </div>

                          {/* Extension assignment */}
                          <div className="space-y-1">
                            {group.extension_ids.length > 0 && (
                              <div className="flex flex-wrap gap-1">
                                {group.extension_ids.map((extId) => {
                                  const ext = extensions.find(
                                    (e) => e.id === extId,
                                  );
                                  if (!ext) return null;
                                  return (
                                    <Badge
                                      key={extId}
                                      variant="secondary"
                                      className="flex items-center gap-1"
                                    >
                                      {ext.name}
                                      <button
                                        type="button"
                                        className="ml-1 hover:text-destructive"
                                        onClick={() =>
                                          handleRemoveFromGroup(group.id, extId)
                                        }
                                      >
                                        ×
                                      </button>
                                    </Badge>
                                  );
                                })}
                              </div>
                            )}
                            {extensions.filter(
                              (e) => !group.extension_ids.includes(e.id),
                            ).length > 0 && (
                              <Select
                                value=""
                                onValueChange={(extId) =>
                                  handleAddToGroup(group.id, extId)
                                }
                              >
                                <SelectTrigger className="h-8 text-xs">
                                  <SelectValue
                                    placeholder={t("extensions.addToGroup")}
                                  />
                                </SelectTrigger>
                                <SelectContent>
                                  {extensions
                                    .filter(
                                      (e) =>
                                        !group.extension_ids.includes(e.id),
                                    )
                                    .map((ext) => (
                                      <SelectItem key={ext.id} value={ext.id}>
                                        {ext.name} (.{ext.file_type})
                                      </SelectItem>
                                    ))}
                                </SelectContent>
                              </Select>
                            )}
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>

          <DialogFooter>
            <RippleButton variant="outline" onClick={onClose}>
              {t("common.buttons.close")}
            </RippleButton>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Delete extension confirmation */}
      <DeleteConfirmationDialog
        isOpen={extensionToDelete !== null}
        onClose={() => setExtensionToDelete(null)}
        onConfirm={handleDeleteExtension}
        title={t("extensions.deleteConfirmTitle")}
        description={t("extensions.deleteConfirmDescription", {
          name: extensionToDelete?.name ?? "",
        })}
        isLoading={isDeleting}
      />

      {/* Delete group confirmation */}
      <DeleteConfirmationDialog
        isOpen={groupToDelete !== null}
        onClose={() => setGroupToDelete(null)}
        onConfirm={handleDeleteGroup}
        title={t("extensions.deleteGroupConfirmTitle")}
        description={t("extensions.deleteGroupConfirmDescription", {
          name: groupToDelete?.name ?? "",
        })}
        isLoading={isDeleting}
      />
    </>
  );
}
