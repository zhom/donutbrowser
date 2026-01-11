"use client";

import { CamoufoxConfigDialog } from "@/components/camoufox-config-dialog";
import { CreateProfileDialog } from "@/components/create-profile-dialog";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { GroupAssignmentDialog } from "@/components/group-assignment-dialog";
import { GroupManagementDialog } from "@/components/group-management-dialog";
import { ImportProfileDialog } from "@/components/import-profile-dialog";
import { PermissionDialog } from "@/components/permission-dialog";
import { ProfileSelectorDialog } from "@/components/profile-selector-dialog";
import { ProfileSyncDialog } from "@/components/profile-sync-dialog";
import { ProxyAssignmentDialog } from "@/components/proxy-assignment-dialog";
import { ProxyManagementDialog } from "@/components/proxy-management-dialog";
import { SettingsDialog } from "@/components/settings-dialog";
import { SyncConfigDialog } from "@/components/sync-config-dialog";
import type { PermissionType } from "@/hooks/use-permissions";
import type { BrowserProfile, CamoufoxConfig, StoredProxy } from "@/types";

type BrowserTypeString =
  | "firefox"
  | "firefox-developer"
  | "chromium"
  | "brave"
  | "zen"
  | "camoufox";

interface PageDialogsProps {
  // Dialog open states
  createProfileDialogOpen: boolean;
  setCreateProfileDialogOpen: (open: boolean) => void;
  importProfileDialogOpen: boolean;
  setImportProfileDialogOpen: (open: boolean) => void;
  settingsDialogOpen: boolean;
  setSettingsDialogOpen: (open: boolean) => void;
  proxyManagementDialogOpen: boolean;
  setProxyManagementDialogOpen: (open: boolean) => void;
  groupManagementDialogOpen: boolean;
  setGroupManagementDialogOpen: (open: boolean) => void;
  syncConfigDialogOpen: boolean;
  setSyncConfigDialogOpen: (open: boolean) => void;
  camoufoxConfigDialogOpen: boolean;
  setCamoufoxConfigDialogOpen: (open: boolean) => void;
  permissionDialogOpen: boolean;
  setPermissionDialogOpen: (open: boolean) => void;
  profileSyncDialogOpen: boolean;
  setProfileSyncDialogOpen: (open: boolean) => void;
  profileSelectorDialogOpen: boolean;
  setProfileSelectorDialogOpen: (open: boolean) => void;
  proxyAssignmentDialogOpen: boolean;
  setProxyAssignmentDialogOpen: (open: boolean) => void;
  groupAssignmentDialogOpen: boolean;
  setGroupAssignmentDialogOpen: (open: boolean) => void;
  deleteConfirmationDialogOpen: boolean;
  setDeleteConfirmationDialogOpen: (open: boolean) => void;

  // Other props
  selectedGroupId: string;
  permissionType: PermissionType | null;
  onPermissionGranted: () => void;
  profiles: BrowserProfile[];
  selectedProfiles: string[];
  storedProxies: StoredProxy[];
  isUpdating: (browser: string) => boolean;
  runningProfiles: Set<string>;
  onCreateProfile: (profileData: {
    name: string;
    browserStr: BrowserTypeString;
    version: string;
    releaseType: string;
    proxyId?: string;
    camoufoxConfig?: CamoufoxConfig;
    groupId?: string;
  }) => Promise<void>;
  onGroupManagementComplete: () => Promise<void>;
  onGroupAssignmentComplete: () => Promise<void>;
  onProxyAssignmentComplete: () => Promise<void>;
  onConfirmDelete: () => Promise<void>;
  deleteTitle: string;
  deleteDescription: string;
  deleteConfirmButtonText: string;
  isDeleting: boolean;
  profileIdsToDelete: string[];
  camoufoxProfile: BrowserProfile | null;
  onCamoufoxSave: (
    profile: BrowserProfile,
    config: CamoufoxConfig,
  ) => Promise<void>;
  profileSyncProfile: BrowserProfile | null;
  onSyncConfigOpen: () => void;
}

export function PageDialogs({
  createProfileDialogOpen,
  setCreateProfileDialogOpen,
  importProfileDialogOpen,
  setImportProfileDialogOpen,
  settingsDialogOpen,
  setSettingsDialogOpen,
  proxyManagementDialogOpen,
  setProxyManagementDialogOpen,
  groupManagementDialogOpen,
  setGroupManagementDialogOpen,
  syncConfigDialogOpen,
  setSyncConfigDialogOpen,
  camoufoxConfigDialogOpen,
  setCamoufoxConfigDialogOpen,
  permissionDialogOpen,
  setPermissionDialogOpen,
  profileSyncDialogOpen,
  setProfileSyncDialogOpen,
  profileSelectorDialogOpen,
  setProfileSelectorDialogOpen,
  proxyAssignmentDialogOpen,
  setProxyAssignmentDialogOpen,
  groupAssignmentDialogOpen,
  setGroupAssignmentDialogOpen,
  deleteConfirmationDialogOpen,
  setDeleteConfirmationDialogOpen,
  selectedGroupId,
  permissionType,
  onPermissionGranted,
  profiles,
  selectedProfiles,
  storedProxies,
  isUpdating,
  runningProfiles,
  onCreateProfile,
  onGroupManagementComplete,
  onGroupAssignmentComplete,
  onProxyAssignmentComplete,
  onConfirmDelete,
  deleteTitle,
  deleteDescription,
  deleteConfirmButtonText,
  isDeleting,
  profileIdsToDelete,
  camoufoxProfile,
  onCamoufoxSave,
  profileSyncProfile,
  onSyncConfigOpen,
}: PageDialogsProps) {
  return (
    <>
      <CreateProfileDialog
        isOpen={createProfileDialogOpen}
        onClose={() => setCreateProfileDialogOpen(false)}
        onCreateProfile={onCreateProfile}
        selectedGroupId={selectedGroupId}
      />
      <ImportProfileDialog
        isOpen={importProfileDialogOpen}
        onClose={() => setImportProfileDialogOpen(false)}
      />
      <SettingsDialog
        isOpen={settingsDialogOpen}
        onClose={() => setSettingsDialogOpen(false)}
      />
      <ProxyManagementDialog
        isOpen={proxyManagementDialogOpen}
        onClose={() => setProxyManagementDialogOpen(false)}
      />
      <GroupManagementDialog
        isOpen={groupManagementDialogOpen}
        onClose={() => setGroupManagementDialogOpen(false)}
        onGroupManagementComplete={onGroupManagementComplete}
      />
      <SyncConfigDialog
        isOpen={syncConfigDialogOpen}
        onClose={() => setSyncConfigDialogOpen(false)}
      />
      <CamoufoxConfigDialog
        isOpen={camoufoxConfigDialogOpen}
        onClose={() => setCamoufoxConfigDialogOpen(false)}
        profile={camoufoxProfile}
        onSave={onCamoufoxSave}
      />
      {permissionType && (
        <PermissionDialog
          isOpen={permissionDialogOpen}
          onClose={() => setPermissionDialogOpen(false)}
          permissionType={permissionType}
          onPermissionGranted={onPermissionGranted}
        />
      )}
      <ProfileSyncDialog
        isOpen={profileSyncDialogOpen}
        onClose={() => setProfileSyncDialogOpen(false)}
        profile={profileSyncProfile}
        onSyncConfigOpen={onSyncConfigOpen}
      />
      <ProfileSelectorDialog
        key={profileSelectorDialogOpen ? "open" : "closed"}
        isOpen={profileSelectorDialogOpen}
        onClose={() => setProfileSelectorDialogOpen(false)}
        url=""
        isUpdating={isUpdating}
        runningProfiles={runningProfiles}
      />
      <ProxyAssignmentDialog
        isOpen={proxyAssignmentDialogOpen}
        onClose={() => setProxyAssignmentDialogOpen(false)}
        selectedProfiles={selectedProfiles}
        onAssignmentComplete={onProxyAssignmentComplete}
        profiles={profiles}
        storedProxies={storedProxies}
      />
      <GroupAssignmentDialog
        isOpen={groupAssignmentDialogOpen}
        onClose={() => setGroupAssignmentDialogOpen(false)}
        selectedProfiles={selectedProfiles}
        onAssignmentComplete={onGroupAssignmentComplete}
        profiles={profiles}
      />
      <ProxyAssignmentDialog
        isOpen={proxyAssignmentDialogOpen}
        onClose={() => setProxyAssignmentDialogOpen(false)}
        selectedProfiles={selectedProfiles}
        onAssignmentComplete={onProxyAssignmentComplete}
        profiles={profiles}
        storedProxies={storedProxies}
      />
      <DeleteConfirmationDialog
        isOpen={deleteConfirmationDialogOpen}
        onClose={() => setDeleteConfirmationDialogOpen(false)}
        onConfirm={onConfirmDelete}
        title={deleteTitle}
        description={deleteDescription}
        confirmButtonText={deleteConfirmButtonText}
        isLoading={isDeleting}
        profileIds={profileIdsToDelete}
        profiles={profiles}
      />
    </>
  );
}
