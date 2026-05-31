"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { FaApple, FaLinux, FaWindows } from "react-icons/fa";
import {
  LuChevronRight,
  LuClipboard,
  LuClipboardCheck,
  LuCookie,
  LuCopy,
  LuFingerprint,
  LuGlobe,
  LuGroup,
  LuKey,
  LuLink,
  LuLock,
  LuLockOpen,
  LuPlus,
  LuPuzzle,
  LuRefreshCw,
  LuSettings,
  LuShield,
  LuShieldCheck,
  LuTrash2,
  LuUpload,
  LuUsers,
  LuX,
} from "react-icons/lu";
import { SharedCamoufoxConfigForm } from "@/components/shared-camoufox-config-form";
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
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { WayfernConfigForm } from "@/components/wayfern-config-form";
import { translateBackendError } from "@/lib/backend-errors";
import { getProfileIcon } from "@/lib/browser-utils";
import { formatRelativeTime } from "@/lib/flag-utils";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type {
  BrowserProfile,
  CamoufoxConfig,
  ProfileGroup,
  StoredProxy,
  VpnConfig,
  WayfernConfig,
} from "@/types";

interface ProfileInfoDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
  storedProxies: StoredProxy[];
  vpnConfigs: VpnConfig[];
  onOpenTrafficDialog?: (profileId: string) => void;
  onOpenProfileSyncDialog?: (profile: BrowserProfile) => void;
  onAssignProfilesToGroup?: (profileIds: string[]) => void;
  onConfigureCamoufox?: (profile: BrowserProfile) => void;
  onCopyCookiesToProfile?: (profile: BrowserProfile) => void;
  onOpenCookieManagement?: (profile: BrowserProfile) => void;
  onAssignExtensionGroup?: (profileIds: string[]) => void;
  onOpenBypassRules?: (profile: BrowserProfile) => void;
  onOpenDnsBlocklist?: (profile: BrowserProfile) => void;
  onOpenLaunchHook?: (profile: BrowserProfile) => void;
  onCloneProfile?: (profile: BrowserProfile) => void;
  onDeleteProfile?: (profile: BrowserProfile) => void;
  onLaunchWithSync?: (profile: BrowserProfile) => void;
  onSetPassword?: (profile: BrowserProfile) => void;
  onChangePassword?: (profile: BrowserProfile) => void;
  onRemovePassword?: (profile: BrowserProfile) => void;
  crossOsUnlocked?: boolean;
  isRunning?: boolean;
  isDisabled?: boolean;
  isCrossOs?: boolean;
  syncStatuses: Record<string, { status: string; error?: string }>;
}

function _OSIcon({ os }: { os: string }) {
  switch (os) {
    case "macos":
      return <FaApple className="size-3.5" />;
    case "windows":
      return <FaWindows className="size-3.5" />;
    case "linux":
      return <FaLinux className="size-3.5" />;
    default:
      return null;
  }
}

function InfoCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md bg-muted/50 border px-3 py-2.5">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="text-sm mt-0.5 truncate">{value}</p>
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/**
 * Shows the total bytes routed through Donut's local proxy worker for this
 * profile. Only counts traffic flowing through the donut-proxy binary — not
 * the browser's full network usage, hence the "Local" qualifier.
 */
function LocalDataTransferCard({
  profileId,
  t,
}: {
  profileId: string;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  type Snapshot = {
    total_bytes_sent: number;
    total_bytes_received: number;
  };
  const [value, setValue] = React.useState<string>("—");

  React.useEffect(() => {
    let mounted = true;
    const fetchSnapshot = async () => {
      try {
        const snap = await invoke<Snapshot | null>(
          "get_profile_traffic_snapshot",
          { profileId },
        );
        if (!mounted) return;
        if (!snap) {
          setValue("0 B");
          return;
        }
        setValue(
          formatBytes(snap.total_bytes_sent + snap.total_bytes_received),
        );
      } catch {
        if (mounted) setValue("—");
      }
    };
    void fetchSnapshot();
    const interval = window.setInterval(fetchSnapshot, 5000);
    return () => {
      mounted = false;
      window.clearInterval(interval);
    };
  }, [profileId]);

  return (
    <InfoCard label={t("profileInfo.fields.localDataTransfer")} value={value} />
  );
}

export function ProfileInfoDialog({
  isOpen,
  onClose,
  profile,
  storedProxies,
  vpnConfigs,
  onOpenTrafficDialog,
  onOpenProfileSyncDialog,
  onAssignProfilesToGroup,
  onConfigureCamoufox,
  onCopyCookiesToProfile,
  onOpenCookieManagement,
  onAssignExtensionGroup,
  onOpenBypassRules,
  onOpenDnsBlocklist,
  onOpenLaunchHook,
  onCloneProfile,
  onDeleteProfile,
  onLaunchWithSync,
  onSetPassword,
  onChangePassword,
  onRemovePassword,
  crossOsUnlocked = false,
  isRunning = false,
  isDisabled = false,
  isCrossOs = false,
  syncStatuses,
}: ProfileInfoDialogProps) {
  const { t } = useTranslation();
  const [copied, setCopied] = React.useState(false);
  const [groupName, setGroupName] = React.useState<string | null>(null);
  const [extensionGroupName, setExtensionGroupName] = React.useState<
    string | null
  >(null);

  React.useEffect(() => {
    if (!isOpen || !profile?.group_id) {
      setGroupName(null);
      return;
    }
    void (async () => {
      try {
        const groups = await invoke<ProfileGroup[]>("get_groups");
        const group = groups.find((g) => g.id === profile.group_id);
        setGroupName(group?.name ?? null);
      } catch {
        setGroupName(null);
      }
    })();
  }, [isOpen, profile?.group_id]);

  React.useEffect(() => {
    if (!isOpen || !profile?.extension_group_id) {
      setExtensionGroupName(null);
      return;
    }
    void (async () => {
      try {
        const group = await invoke<{ name: string } | null>(
          "get_extension_group_for_profile",
          { profileId: profile.id },
        );
        setExtensionGroupName(group?.name ?? null);
      } catch {
        setExtensionGroupName(null);
      }
    })();
  }, [isOpen, profile?.extension_group_id, profile?.id]);

  React.useEffect(() => {
    if (!isOpen) {
      setCopied(false);
    }
  }, [isOpen]);

  if (!profile) return null;

  const ProfileIcon = getProfileIcon(profile);
  const isCamoufoxOrWayfern =
    profile.browser === "camoufox" || profile.browser === "wayfern";
  const isDeleteDisabled = isRunning;

  const proxyName = profile.proxy_id
    ? storedProxies.find((p) => p.id === profile.proxy_id)?.name
    : null;
  const vpnName = profile.vpn_id
    ? vpnConfigs.find((v) => v.id === profile.vpn_id)?.name
    : null;
  const networkLabel = vpnName
    ? `VPN: ${vpnName}`
    : proxyName
      ? `Proxy: ${proxyName}`
      : t("profileInfo.values.none");

  const syncStatus = syncStatuses[profile.id];
  const syncMode = profile.sync_mode ?? "Disabled";

  const handleCopyId = async () => {
    try {
      await navigator.clipboard.writeText(profile.id);
      setCopied(true);
      setTimeout(() => {
        setCopied(false);
      }, 2000);
    } catch {
      // ignore
    }
  };

  const handleAction = (action: () => void) => {
    onClose();
    action();
  };

  const hasTags = profile.tags && profile.tags.length > 0;
  const hasNote = !!profile.note;

  // Items in the settings tab `actions` list MUST only open another dialog
  // (or trigger a navigation/action that closes this one). Do NOT put inline
  // settings UI — inputs, toggles, save buttons — directly in this dialog's
  // settings tab. Each setting belongs in its own focused dialog (see
  // `ProfileLaunchHookDialog`, `ProfileBypassRulesDialog`,
  // `ProfileDnsBlocklistDialog` for the pattern). The settings tab is purely
  // a navigation hub.
  interface ActionItem {
    icon: React.ReactNode;
    label: string;
    onClick: () => void;
    disabled?: boolean;
    destructive?: boolean;
    proBadge?: boolean;
    runningBadge?: boolean;
    hidden?: boolean;
  }

  const actions: ActionItem[] = [
    {
      icon: <LuGlobe className="size-4" />,
      label: t("profiles.actions.viewNetwork"),
      onClick: () => {
        handleAction(() => onOpenTrafficDialog?.(profile.id));
      },
      disabled: isCrossOs,
    },
    {
      icon: <LuRefreshCw className="size-4" />,
      label: t("profiles.actions.syncSettings"),
      onClick: () => {
        handleAction(() => onOpenProfileSyncDialog?.(profile));
      },
      disabled: isCrossOs,
      hidden: profile.ephemeral === true,
    },
    {
      icon: <LuGroup className="size-4" />,
      label: t("profiles.actions.assignToGroup"),
      onClick: () => {
        handleAction(() => onAssignProfilesToGroup?.([profile.id]));
      },
      disabled: isDisabled,
      runningBadge: isRunning,
    },
    {
      icon: <LuFingerprint className="size-4" />,
      label: t("profiles.actions.changeFingerprint"),
      onClick: () => {
        handleAction(() => onConfigureCamoufox?.(profile));
      },
      // Viewing and editing fingerprints both require an active paid plan.
      disabled: isDisabled || !crossOsUnlocked,
      proBadge: !crossOsUnlocked,
      runningBadge: isRunning,
      hidden: !isCamoufoxOrWayfern || !onConfigureCamoufox,
    },
    {
      icon: <LuUsers className="size-4" />,
      label: t("profiles.synchronizer.launchWithSync"),
      onClick: () => {
        handleAction(() => onLaunchWithSync?.(profile));
      },
      disabled: isDisabled || isRunning || !crossOsUnlocked,
      proBadge: !crossOsUnlocked,
      hidden: profile.browser !== "wayfern" || !onLaunchWithSync,
    },
    {
      icon: <LuCopy className="size-4" />,
      label: t("profiles.actions.copyCookiesToProfile"),
      onClick: () => {
        handleAction(() => onCopyCookiesToProfile?.(profile));
      },
      disabled: isDisabled,
      runningBadge: isRunning,
      hidden:
        !isCamoufoxOrWayfern ||
        profile.ephemeral === true ||
        !onCopyCookiesToProfile,
    },
    {
      icon: <LuCookie className="size-4" />,
      label: t("profileInfo.actions.manageCookies"),
      onClick: () => {
        handleAction(() => onOpenCookieManagement?.(profile));
      },
      disabled: isDisabled,
      runningBadge: isRunning,
      hidden:
        !isCamoufoxOrWayfern ||
        profile.ephemeral === true ||
        !onOpenCookieManagement,
    },
    {
      icon: <LuSettings className="size-4" />,
      label: t("profiles.actions.clone"),
      onClick: () => {
        handleAction(() => onCloneProfile?.(profile));
      },
      disabled: isDisabled,
      runningBadge: isRunning,
      hidden: profile.ephemeral === true,
    },
    {
      icon: <LuPuzzle className="size-4" />,
      label: t("profileInfo.actions.assignExtensionGroup"),
      onClick: () => {
        handleAction(() => onAssignExtensionGroup?.([profile.id]));
      },
      disabled: isDisabled,
      runningBadge: isRunning,
      hidden: profile.ephemeral === true,
    },
    {
      icon: <LuShieldCheck className="size-4" />,
      label: t("profileInfo.network.bypassRulesTitle"),
      onClick: () => {
        handleAction(() => onOpenBypassRules?.(profile));
      },
    },
    {
      icon: <LuShield className="size-4" />,
      label: t("dnsBlocklist.title"),
      onClick: () => {
        handleAction(() => onOpenDnsBlocklist?.(profile));
      },
    },
    {
      icon: <LuLink className="size-4" />,
      label: t("profiles.actions.launchHook"),
      onClick: () => {
        handleAction(() => onOpenLaunchHook?.(profile));
      },
      hidden: !onOpenLaunchHook,
    },
    {
      icon: <LuKey className="size-4" />,
      label: t("profiles.actions.setPassword"),
      onClick: () => {
        handleAction(() => onSetPassword?.(profile));
      },
      disabled: isDisabled || isRunning,
      runningBadge: isRunning,
      hidden:
        profile.password_protected === true ||
        profile.ephemeral === true ||
        !onSetPassword,
    },
    {
      icon: <LuKey className="size-4" />,
      label: t("profiles.actions.changePassword"),
      onClick: () => {
        handleAction(() => onChangePassword?.(profile));
      },
      disabled: isDisabled || isRunning,
      runningBadge: isRunning,
      hidden: profile.password_protected !== true || !onChangePassword,
    },
    {
      icon: <LuLockOpen className="size-4" />,
      label: t("profiles.actions.removePassword"),
      onClick: () => {
        handleAction(() => onRemovePassword?.(profile));
      },
      disabled: isDisabled || isRunning,
      runningBadge: isRunning,
      hidden: profile.password_protected !== true || !onRemovePassword,
      destructive: true,
    },
    {
      icon: <LuTrash2 className="size-4" />,
      label: t("profiles.actions.delete"),
      onClick: () => {
        handleAction(() => onDeleteProfile?.(profile));
      },
      disabled: isDeleteDisabled,
      destructive: true,
    },
  ];

  const visibleActions = actions.filter((a) => !a.hidden);

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent
        hideClose
        className="sm:max-w-3xl w-[720px] max-w-[720px] h-[480px] max-h-[480px] flex flex-col p-0 gap-0 overflow-hidden"
      >
        {/* The dialog renders its own custom header, so the accessible title is
            visually hidden but present for screen readers (Radix requires it). */}
        <DialogTitle className="sr-only">{t("profileInfo.title")}</DialogTitle>
        <ProfileInfoLayout
          profile={profile}
          ProfileIcon={ProfileIcon}
          isRunning={isRunning}
          isDisabled={isDisabled}
          networkLabel={networkLabel}
          groupName={groupName}
          extensionGroupName={extensionGroupName}
          syncMode={syncMode}
          syncStatus={syncStatus}
          storedProxies={storedProxies}
          vpnConfigs={vpnConfigs}
          hasTags={hasTags}
          hasNote={hasNote}
          copied={copied}
          handleCopyId={handleCopyId}
          onClose={onClose}
          onCloneProfile={onCloneProfile}
          onKillProfile={undefined}
          visibleActions={visibleActions}
          t={t}
        />
      </DialogContent>
    </Dialog>
  );
}

interface ProfileInfoLayoutProps {
  profile: BrowserProfile;
  ProfileIcon: React.ComponentType<{ className?: string }>;
  isRunning: boolean;
  isDisabled: boolean;
  networkLabel: string;
  groupName: string | null;
  extensionGroupName: string | null;
  syncMode: string;
  syncStatus: { status: string; error?: string } | undefined;
  hasTags: boolean | undefined;
  hasNote: boolean;
  copied: boolean;
  storedProxies: StoredProxy[];
  vpnConfigs: VpnConfig[];
  handleCopyId: () => Promise<void>;
  onClose: () => void;
  onCloneProfile?: (profile: BrowserProfile) => void;
  onKillProfile?: (profile: BrowserProfile) => void;
  visibleActions: {
    icon: React.ReactNode;
    label: string;
    onClick: () => void;
    disabled?: boolean;
    destructive?: boolean;
    proBadge?: boolean;
    runningBadge?: boolean;
  }[];
  t: (key: string, options?: Record<string, unknown>) => string;
}

type ProfileSection =
  | "overview"
  | "fingerprint"
  | "network"
  | "cookies"
  | "extensions"
  | "sync"
  | "automation"
  | "security"
  | "delete";

function ProfileInfoLayout({
  profile,
  ProfileIcon,
  isRunning,
  isDisabled,
  networkLabel,
  groupName,
  extensionGroupName,
  syncMode,
  syncStatus,
  storedProxies,
  vpnConfigs,
  hasTags,
  hasNote,
  copied,
  handleCopyId,
  onClose,
  onCloneProfile,
  visibleActions,
  t,
}: ProfileInfoLayoutProps) {
  const [section, setSection] = React.useState<ProfileSection>("overview");

  // Map sidebar items to existing action labels, so clicking a section
  // simply triggers the existing dialog handler.
  const findAction = React.useCallback(
    (substr: string) =>
      visibleActions.find((a) => a.label.toLowerCase().includes(substr)),
    [visibleActions],
  );

  const deleteAction = findAction("delete");
  const fingerprintAction = findAction("fingerprint");
  const cookiesManageAction = findAction("manage cookies");
  const cookiesCopyAction = findAction("copy cookies");
  const cookiesAction = cookiesManageAction ?? cookiesCopyAction;
  const extensionAction = findAction("extension");
  const syncAction = findAction("sync");
  const _launchHookAction = findAction("hook") ?? findAction("launch hook");
  const _networkAction = findAction("network");
  // Password actions are no longer routed via the legacy action handlers —
  // SecuritySectionInline writes directly to the backend instead.

  // Cookie count is fetched at the layout level so the sidebar badge can
  // surface it without waiting for the user to open the Cookies section.
  // The effect deps must be primitive — `cookiesAction` is a new object
  // every render and using it directly here caused an infinite re-render
  // loop that froze the entire app when the dialog opened.
  // Skipped while running: the cookie DB is held by the browser and we
  // don't want to compete for its lock from the badge fetch.
  const cookiesSupported = !!cookiesAction;
  const [cookieCount, setCookieCount] = React.useState<number | null>(null);
  React.useEffect(() => {
    if (!cookiesSupported || isRunning) {
      setCookieCount(null);
      return;
    }
    let mounted = true;
    void (async () => {
      try {
        const data = await invoke<{ total_count: number }>(
          "get_profile_cookie_stats",
          { profileId: profile.id },
        );
        if (mounted) setCookieCount(data.total_count);
      } catch {
        if (mounted) setCookieCount(null);
      }
    })();
    return () => {
      mounted = false;
    };
  }, [profile.id, cookiesSupported, isRunning]);

  const sidebarItems: {
    id: ProfileSection;
    icon: React.ReactNode;
    label: string;
    badge?: string;
    destructive?: boolean;
    hidden?: boolean;
  }[] = [
    {
      id: "overview",
      icon: <LuClipboard className="size-3.5" />,
      label: t("profileInfo.sections.overview"),
    },
    {
      id: "fingerprint",
      icon: <LuFingerprint className="size-3.5" />,
      label: t("profileInfo.sections.fingerprint"),
      badge: profile.password_protected
        ? t("profileInfo.badges.locked")
        : undefined,
      hidden: !fingerprintAction,
    },
    {
      id: "network",
      icon: <LuGlobe className="size-3.5" />,
      label: t("profileInfo.sections.network"),
      badge: profile.proxy_id || profile.vpn_id ? networkLabel : undefined,
    },
    {
      id: "cookies",
      icon: <LuCookie className="size-3.5" />,
      label: t("profileInfo.sections.cookies"),
      badge:
        cookieCount !== null && cookieCount > 0
          ? cookieCount.toLocaleString()
          : undefined,
      hidden: !cookiesAction,
    },
    {
      id: "extensions",
      icon: <LuPuzzle className="size-3.5" />,
      label: t("profileInfo.sections.extensions"),
      badge: extensionGroupName ?? undefined,
      hidden: !extensionAction,
    },
    {
      id: "sync",
      icon: <LuRefreshCw className="size-3.5" />,
      label: t("profileInfo.sections.sync"),
      hidden: !syncAction,
    },
    {
      id: "automation",
      icon: <LuLink className="size-3.5" />,
      label: t("profileInfo.sections.launchHook"),
      badge: profile.launch_hook ? t("profileInfo.badges.active") : undefined,
    },
    {
      id: "security",
      icon: <LuKey className="size-3.5" />,
      label: t("profileInfo.sections.security"),
    },
  ];

  return (
    <>
      {/* Top bar */}
      <div className="flex items-center gap-2 h-11 px-3 border-b border-border shrink-0">
        <LuUsers className="size-3.5 text-muted-foreground shrink-0" />
        <div className="flex items-center gap-1.5 text-xs min-w-0 flex-1">
          <span className="font-semibold">
            {t("profileInfo.breadcrumbRoot")}
          </span>
          <span className="text-muted-foreground">/</span>
          <span className="text-muted-foreground truncate">{profile.name}</span>
        </div>
        {onCloneProfile && (
          <Button
            variant="ghost"
            size="sm"
            className="h-7 px-2 text-xs gap-1.5"
            disabled={isDisabled}
            onClick={() => onCloneProfile(profile)}
          >
            <LuCopy className="size-3" />
            {t("profileInfo.duplicate")}
          </Button>
        )}
        <button
          type="button"
          aria-label={t("common.buttons.close")}
          onClick={onClose}
          className="grid place-items-center size-7 rounded-md text-muted-foreground hover:text-foreground hover:bg-accent/50 transition-colors duration-100"
        >
          <LuX className="size-3.5" />
        </button>
      </div>

      {/* Body */}
      <div className="flex flex-1 min-h-0">
        {/* Sidebar */}
        <nav className="w-44 shrink-0 border-r border-border p-2 flex flex-col gap-0.5 overflow-y-auto">
          {sidebarItems
            .filter((it) => !it.hidden)
            .map((it) => {
              const active = section === it.id;
              return (
                <button
                  key={it.id}
                  type="button"
                  onClick={() => setSection(it.id)}
                  className={cn(
                    "flex items-center gap-2 h-7 px-2 rounded-md text-xs transition-colors duration-100 text-left",
                    active
                      ? "bg-accent text-accent-foreground"
                      : "text-muted-foreground hover:text-foreground hover:bg-accent/50",
                  )}
                >
                  <span className="shrink-0">{it.icon}</span>
                  <span className="flex-1 truncate">{it.label}</span>
                  {it.badge && (
                    <span className="text-[9px] uppercase text-muted-foreground tracking-wide truncate max-w-[60px]">
                      {it.badge}
                    </span>
                  )}
                </button>
              );
            })}
          {deleteAction && (
            <>
              <div className="my-1 h-px bg-border" />
              <button
                type="button"
                onClick={deleteAction.onClick}
                disabled={deleteAction.disabled}
                className="flex items-center gap-2 h-7 px-2 rounded-md text-xs transition-colors duration-100 text-destructive hover:bg-destructive/10 disabled:opacity-50 disabled:pointer-events-none"
              >
                <LuTrash2 className="size-3.5 shrink-0" />
                <span className="flex-1 text-left">
                  {t("profileInfo.sections.delete")}
                </span>
              </button>
            </>
          )}
        </nav>

        {/* Main */}
        <div className="flex-1 min-w-0 overflow-y-auto scroll-fade p-4">
          {section === "overview" && (
            <div className="flex flex-col gap-3">
              {/* Hero */}
              <div className="flex items-center gap-3">
                <div className="rounded-lg bg-muted p-2.5 shrink-0">
                  <ProfileIcon className="size-7 text-foreground" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <h3 className="text-base font-semibold truncate">
                      {profile.name}
                    </h3>
                  </div>
                  <div className="flex flex-wrap items-center gap-1.5 mt-1 text-[11px]">
                    <span className="font-mono text-muted-foreground">
                      {profile.version}
                    </span>
                  </div>
                </div>
              </div>

              {/* ID */}
              <div className="flex items-center gap-2 rounded-md bg-muted/40 px-3 py-2 border border-border">
                <span className="text-[10px] uppercase tracking-wide text-muted-foreground shrink-0">
                  ID
                </span>
                <span className="font-mono text-xs truncate flex-1">
                  {profile.id}
                </span>
                <button
                  type="button"
                  onClick={() => void handleCopyId()}
                  className="text-muted-foreground hover:text-foreground transition-colors shrink-0"
                  aria-label={t("common.buttons.copy")}
                >
                  {copied ? (
                    <LuClipboardCheck className="size-3.5" />
                  ) : (
                    <LuClipboard className="size-3.5" />
                  )}
                </button>
              </div>

              {/* 2x2 cards */}
              <div className="grid grid-cols-2 gap-2">
                <InfoCard
                  label={t("profileInfo.fields.group")}
                  value={groupName ?? t("profileInfo.values.none")}
                />
                <InfoCard
                  label={t("profileInfo.fields.proxyVpn")}
                  value={networkLabel}
                />
                <InfoCard
                  label={t("profileInfo.fields.tags")}
                  value={
                    hasTags
                      ? (profile.tags ?? []).join(", ")
                      : t("profileInfo.values.none")
                  }
                />
                <InfoCard
                  label={t("profileInfo.fields.note")}
                  value={
                    hasNote
                      ? (profile.note ?? "")
                      : t("profileInfo.values.none")
                  }
                />
              </div>

              {/* Activity */}
              <div className="mt-1 flex flex-col gap-1.5">
                <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
                  {t("profileInfo.sections.activity")}
                </span>
                <div className="grid grid-cols-2 gap-2">
                  <InfoCard
                    label={t("profileInfo.fields.lastLaunched")}
                    value={
                      isRunning
                        ? t("profileInfo.values.activeNow")
                        : profile.last_launch
                          ? formatRelativeTime(profile.last_launch)
                          : t("profileInfo.values.never")
                    }
                  />
                  <LocalDataTransferCard profileId={profile.id} t={t} />
                </div>
              </div>

              {profile.created_by_email && (
                <div className="rounded-md bg-muted/40 border border-border px-3 py-2">
                  <p className="text-[10px] uppercase tracking-wide text-muted-foreground">
                    {t("sync.team.title")}
                  </p>
                  <p className="text-sm mt-0.5">
                    {t("sync.team.createdBy", {
                      email: profile.created_by_email,
                    })}
                  </p>
                </div>
              )}
            </div>
          )}

          {section === "fingerprint" && (
            <FingerprintSectionInline
              profile={profile}
              isDisabled={isDisabled}
              crossOsUnlocked={Boolean(
                // Re-derive: parent passes crossOsUnlocked but the layout
                // doesn't get it; we get it implicitly via fingerprintAction's
                // proBadge state. Default to false if action missing.
                fingerprintAction && !fingerprintAction.proBadge,
              )}
              onSaved={onClose}
              t={t}
            />
          )}

          {section === "network" && (
            <NetworkSectionInline
              profile={profile}
              storedProxies={storedProxies}
              vpnConfigs={vpnConfigs}
              isDisabled={isDisabled}
              t={t}
            />
          )}

          {section === "cookies" && (
            <CookiesSectionInline
              profile={profile}
              isRunning={isRunning}
              isDisabled={isDisabled}
              onCopyCookies={cookiesCopyAction?.onClick}
              onImportCookies={cookiesManageAction?.onClick}
              t={t}
            />
          )}

          {section === "extensions" && (
            <ExtensionsSectionInline
              profile={profile}
              isDisabled={isDisabled}
              t={t}
            />
          )}

          {section === "sync" && (
            <SyncSectionInline
              profile={profile}
              syncMode={syncMode}
              syncStatus={syncStatus}
              isDisabled={isDisabled}
              t={t}
            />
          )}

          {section === "automation" && (
            <LaunchHookEditor profile={profile} t={t} />
          )}

          {section === "security" && (
            <SecuritySectionInline
              profile={profile}
              isRunning={isRunning}
              t={t}
            />
          )}
        </div>
      </div>
    </>
  );
}

function _SectionPlaceholder({
  icon,
  title,
  description,
  actionLabel,
  onAction,
  disabled,
  hint,
}: {
  icon: React.ReactNode;
  title: string;
  description: string;
  actionLabel: string;
  onAction: () => void;
  disabled?: boolean;
  hint?: string;
}) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 text-sm font-semibold">
        {icon}
        {title}
      </div>
      <p className="text-xs text-muted-foreground">{description}</p>
      {hint && (
        <div className="rounded-md bg-muted/40 border border-border px-3 py-2 text-xs">
          {hint}
        </div>
      )}
      <Button
        size="sm"
        onClick={onAction}
        disabled={disabled}
        className="self-start h-7 text-xs"
      >
        {actionLabel}
      </Button>
    </div>
  );
}

function _SectionAction({
  icon,
  label,
  onClick,
  disabled,
  destructive,
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  destructive?: boolean;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "flex items-center gap-2 h-9 px-3 rounded-md text-xs transition-colors text-left",
        destructive
          ? "text-destructive hover:bg-destructive/10"
          : "hover:bg-accent",
        "disabled:opacity-50 disabled:pointer-events-none",
      )}
    >
      {icon}
      <span className="flex-1">{label}</span>
      <LuChevronRight className="size-3.5 text-muted-foreground" />
    </button>
  );
}

function isValidHttpUrl(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return false;
  try {
    const u = new URL(trimmed);
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return false;
  }
}

function LaunchHookEditor({
  profile,
  t,
}: {
  profile: BrowserProfile;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  const { t: tFn } = useTranslation();
  const [value, setValue] = React.useState(profile.launch_hook ?? "");
  const [isSaving, setIsSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const initial = profile.launch_hook ?? "";
  const dirty = value !== initial;
  const trimmed = value.trim();
  const showInvalidHint = trimmed.length > 0 && !isValidHttpUrl(trimmed);

  const onSave = async () => {
    setIsSaving(true);
    setError(null);
    try {
      await invoke("update_profile_launch_hook", {
        profileId: profile.id,
        launchHook: trimmed ? trimmed : null,
      });
    } catch (e) {
      setError(translateBackendError(tFn, e));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 text-sm font-semibold">
        <LuLink className="size-4" />
        {t("profileInfo.sections.launchHook")}
      </div>
      <p className="text-xs text-muted-foreground">
        {t("profileInfo.sectionDesc.launchHook")}
      </p>
      <Input
        type="url"
        value={value}
        onChange={(e) => {
          setValue(e.target.value);
        }}
        placeholder={t("profiles.launchHook.placeholder")}
        className="text-xs font-mono"
      />
      {showInvalidHint && (
        <p className="text-xs text-warning">
          {t("profileInfo.launchHook.invalidUrlHint")}
        </p>
      )}
      {error && <p className="text-xs text-destructive">{error}</p>}
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          className="h-7 text-xs"
          disabled={!dirty || isSaving || showInvalidHint}
          onClick={() => {
            void onSave();
          }}
        >
          {isSaving ? t("common.buttons.saving") : t("common.buttons.save")}
        </Button>
        {dirty && (
          <Button
            size="sm"
            variant="ghost"
            className="h-7 text-xs"
            onClick={() => {
              setValue(initial);
              setError(null);
            }}
          >
            {t("common.buttons.cancel")}
          </Button>
        )}
      </div>
    </div>
  );
}

function SyncSectionInline({
  profile,
  syncMode,
  syncStatus,
  isDisabled,
  t,
}: {
  profile: BrowserProfile;
  syncMode: string;
  syncStatus: { status: string; error?: string } | undefined;
  isDisabled: boolean;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  const [isSaving, setIsSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const onChangeMode = async (mode: string) => {
    setIsSaving(true);
    setError(null);
    try {
      await invoke("set_profile_sync_mode", {
        profileId: profile.id,
        syncMode: mode,
      });
    } catch (e) {
      setError(String(e));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 text-sm font-semibold">
        <LuRefreshCw className="size-4" />
        {t("profileInfo.sections.sync")}
      </div>
      <p className="text-xs text-muted-foreground">
        {t("profileInfo.sectionDesc.sync")}
      </p>
      <div className="flex items-center gap-2">
        <span className="text-[10px] uppercase tracking-wide text-muted-foreground shrink-0">
          {t("profileInfo.fields.syncMode")}
        </span>
        <Select
          value={syncMode}
          disabled={isDisabled || isSaving}
          onValueChange={(v) => {
            void onChangeMode(v);
          }}
        >
          <SelectTrigger className="h-7 text-xs flex-1">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="Disabled">{t("sync.mode.disabled")}</SelectItem>
            <SelectItem value="Regular">{t("sync.mode.regular")}</SelectItem>
            <SelectItem value="Encrypted">
              {t("sync.mode.encrypted")}
            </SelectItem>
          </SelectContent>
        </Select>
      </div>
      {syncStatus && (
        <div className="rounded-md bg-muted/40 border border-border px-3 py-2">
          <p className="text-[10px] uppercase tracking-wide text-muted-foreground">
            {t("profileInfo.fields.syncStatus")}
          </p>
          <p className="text-sm mt-0.5">{syncStatus.status}</p>
          {syncStatus.error && (
            <p className="text-xs text-destructive mt-1">{syncStatus.error}</p>
          )}
        </div>
      )}
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  );
}

function NetworkSectionInline({
  profile,
  storedProxies,
  vpnConfigs,
  isDisabled,
  t,
}: {
  profile: BrowserProfile;
  storedProxies: StoredProxy[];
  vpnConfigs: VpnConfig[];
  isDisabled: boolean;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  const [isSaving, setIsSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  // Track the effective selection so the dropdown reflects the last save
  // even before the parent `profile` prop refreshes from a backend event.
  const [proxyId, setProxyId] = React.useState<string | null>(
    profile.proxy_id ?? null,
  );
  const [vpnId, setVpnId] = React.useState<string | null>(
    profile.vpn_id ?? null,
  );

  React.useEffect(() => {
    setProxyId(profile.proxy_id ?? null);
    setVpnId(profile.vpn_id ?? null);
  }, [profile.proxy_id, profile.vpn_id]);

  const onProxyChange = async (value: string) => {
    const nextId = value === "__none__" ? null : value;
    setIsSaving(true);
    setError(null);
    try {
      await invoke("update_profile_proxy", {
        profileId: profile.id,
        proxyId: nextId,
      });
      // Clearing the proxy implicitly clears any VPN binding too on the
      // backend, but we mirror it locally for an immediate visual.
      setProxyId(nextId);
      if (nextId !== null) setVpnId(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setIsSaving(false);
    }
  };

  const onVpnChange = async (value: string) => {
    const nextId = value === "__none__" ? null : value;
    setIsSaving(true);
    setError(null);
    try {
      await invoke("update_profile_vpn", {
        profileId: profile.id,
        vpnId: nextId,
      });
      setVpnId(nextId);
      if (nextId !== null) setProxyId(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 text-sm font-semibold">
        <LuGlobe className="size-4" />
        {t("profileInfo.sections.network")}
      </div>
      <p className="text-xs text-muted-foreground">
        {t("profileInfo.sectionDesc.network")}
      </p>

      <div className="flex items-center gap-2">
        <span className="text-[10px] uppercase tracking-wide text-muted-foreground shrink-0 w-12">
          {t("profileInfo.fields.proxy")}
        </span>
        <Select
          value={proxyId ?? "__none__"}
          disabled={isDisabled || isSaving}
          onValueChange={(v) => {
            void onProxyChange(v);
          }}
        >
          <SelectTrigger className="h-7 text-xs flex-1">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__none__">
              {t("profileInfo.values.none")}
            </SelectItem>
            {storedProxies.map((p) => (
              <SelectItem key={p.id} value={p.id}>
                {p.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="flex items-center gap-2">
        <span className="text-[10px] uppercase tracking-wide text-muted-foreground shrink-0 w-12">
          {t("profileInfo.fields.vpn")}
        </span>
        <Select
          value={vpnId ?? "__none__"}
          disabled={isDisabled || isSaving}
          onValueChange={(v) => {
            void onVpnChange(v);
          }}
        >
          <SelectTrigger className="h-7 text-xs flex-1">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__none__">
              {t("profileInfo.values.none")}
            </SelectItem>
            {vpnConfigs.map((v) => (
              <SelectItem key={v.id} value={v.id}>
                {v.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  );
}

function ExtensionsSectionInline({
  profile,
  isDisabled,
  t,
}: {
  profile: BrowserProfile;
  isDisabled: boolean;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  type ExtensionGroupOption = { id: string; name: string };
  const [groups, setGroups] = React.useState<ExtensionGroupOption[]>([]);
  const [groupId, setGroupId] = React.useState<string | null>(
    profile.extension_group_id ?? null,
  );
  const [isSaving, setIsSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    setGroupId(profile.extension_group_id ?? null);
  }, [profile.extension_group_id]);

  React.useEffect(() => {
    let mounted = true;
    let unlisten: (() => void) | undefined;
    const load = async () => {
      try {
        const data = await invoke<ExtensionGroupOption[]>(
          "list_extension_groups",
        );
        if (mounted) setGroups(data);
      } catch (e) {
        if (mounted) setError(String(e));
      }
    };
    void load();
    void listen("extensions-changed", () => {
      void load();
    }).then((u) => {
      if (mounted) unlisten = u;
      else u();
    });
    return () => {
      mounted = false;
      unlisten?.();
    };
  }, []);

  const onChange = async (value: string) => {
    const next = value === "__none__" ? null : value;
    setIsSaving(true);
    setError(null);
    try {
      await invoke("assign_extension_group_to_profile", {
        profileId: profile.id,
        extensionGroupId: next,
      });
      setGroupId(next);
    } catch (e) {
      setError(String(e));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 text-sm font-semibold">
        <LuPuzzle className="size-4" />
        {t("profileInfo.sections.extensions")}
      </div>
      <p className="text-xs text-muted-foreground">
        {t("profileInfo.sectionDesc.extensions")}
      </p>
      <div className="flex items-center gap-2">
        <span className="text-[10px] uppercase tracking-wide text-muted-foreground shrink-0 w-16">
          {t("profileInfo.fields.extensionGroup")}
        </span>
        <Select
          value={groupId ?? "__none__"}
          disabled={isDisabled || isSaving}
          onValueChange={(v) => {
            void onChange(v);
          }}
        >
          <SelectTrigger className="h-7 text-xs flex-1">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__none__">
              {t("profileInfo.values.none")}
            </SelectItem>
            {groups.map((g) => (
              <SelectItem key={g.id} value={g.id}>
                {g.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  );
}

function CookiesSectionInline({
  profile,
  isRunning,
  isDisabled,
  onCopyCookies,
  onImportCookies,
  t,
}: {
  profile: BrowserProfile;
  isRunning: boolean;
  isDisabled: boolean;
  onCopyCookies?: () => void;
  onImportCookies?: () => void;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  type CookieStats = {
    profile_id: string;
    browser_type: string;
    total_count: number;
    domains: { domain: string; count: number }[];
  };
  const [stats, setStats] = React.useState<CookieStats | null>(null);
  const [isLoading, setIsLoading] = React.useState(!isRunning);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (isRunning) {
      setStats(null);
      setIsLoading(false);
      setError(null);
      return;
    }
    let mounted = true;
    setIsLoading(true);
    setError(null);
    void (async () => {
      try {
        const data = await invoke<CookieStats>("get_profile_cookie_stats", {
          profileId: profile.id,
        });
        if (mounted) setStats(data);
      } catch (e) {
        if (mounted) setError(translateBackendError(t as never, e));
      } finally {
        if (mounted) setIsLoading(false);
      }
    })();
    return () => {
      mounted = false;
    };
  }, [profile.id, isRunning, t]);

  const domains = stats?.domains ?? [];

  return (
    <div className="flex flex-col gap-3 min-h-0 flex-1">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <LuCookie className="size-4" />
          {t("profileInfo.sections.cookies")}
        </div>
        <div className="flex items-center gap-2">
          {onImportCookies && (
            <Button
              variant="outline"
              size="sm"
              className="h-7 gap-1.5"
              disabled={isDisabled || isRunning}
              onClick={onImportCookies}
            >
              <LuUpload className="size-3.5" />
              {t("cookies.import.title")}
            </Button>
          )}
          {onCopyCookies && (
            <Button
              variant="outline"
              size="sm"
              className="h-7 gap-1.5"
              disabled={isDisabled}
              onClick={onCopyCookies}
            >
              <LuCopy className="size-3.5" />
              {t("profiles.actions.copyCookies")}
            </Button>
          )}
        </div>
      </div>
      <p className="text-xs text-muted-foreground">
        {t("profileInfo.sectionDesc.cookies")}
      </p>
      {isRunning ? (
        <div className="rounded-md bg-muted/40 border border-border px-3 py-2">
          <p className="text-xs text-muted-foreground">
            {t("profileInfo.cookies.runningNotice")}
          </p>
        </div>
      ) : (
        <>
          <div className="rounded-md bg-muted/40 border border-border px-3 py-2">
            <p className="text-[10px] uppercase tracking-wide text-muted-foreground">
              {t("profileInfo.fields.cookieCount")}
            </p>
            <p className="text-sm mt-0.5">
              {isLoading
                ? t("profileInfo.values.loading")
                : stats
                  ? stats.total_count.toLocaleString()
                  : "—"}
            </p>
          </div>
          {domains.length > 0 && (
            <div className="rounded-md bg-muted/40 border border-border flex flex-col min-h-0 flex-1 overflow-hidden">
              <p className="text-[10px] uppercase tracking-wide text-muted-foreground px-3 py-2 border-b border-border shrink-0">
                {t("profileInfo.cookies.domainsHeader", {
                  count: domains.length,
                })}
              </p>
              <ul className="text-xs px-3 py-2 overflow-y-auto flex-1 space-y-1">
                {domains.map((d) => (
                  <li
                    key={d.domain}
                    className="flex items-center justify-between gap-2"
                  >
                    <span className="truncate font-mono">{d.domain}</span>
                    <span className="text-muted-foreground tabular-nums">
                      {d.count}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {error && <p className="text-xs text-destructive">{error}</p>}
        </>
      )}
    </div>
  );
}

// Inline password set / change / remove form. Replaces three separate
// nested modal dialogs with one in-page form that branches on the current
// `password_protected` state of the profile.
// Inline fingerprint editor. Reuses SharedCamoufoxConfigForm (Camoufox/Firefox
// engine) and WayfernConfigForm (Chromium engine) so the same field set as
// the standalone dialog is available without opening a nested modal.
function FingerprintSectionInline({
  profile,
  isDisabled,
  crossOsUnlocked,
  onSaved,
  t,
}: {
  profile: BrowserProfile;
  isDisabled: boolean;
  crossOsUnlocked: boolean;
  onSaved: () => void;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  const [camoufoxConfig, setCamoufoxConfig] = React.useState<CamoufoxConfig>(
    () => profile.camoufox_config ?? {},
  );
  const [wayfernConfig, setWayfernConfig] = React.useState<WayfernConfig>(
    () => profile.wayfern_config ?? {},
  );
  const [isSaving, setIsSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [success, setSuccess] = React.useState<string | null>(null);

  // When the underlying profile changes (e.g. a different profile is opened
  // in the dialog) reset the local form state to match.
  React.useEffect(() => {
    setCamoufoxConfig(profile.camoufox_config ?? {});
    setWayfernConfig(profile.wayfern_config ?? {});
    setError(null);
    setSuccess(null);
  }, [profile.camoufox_config, profile.wayfern_config]);

  const isCamoufox = profile.browser === "camoufox";
  const isWayfern = profile.browser === "wayfern";

  if (!isCamoufox && !isWayfern) {
    return (
      <div className="flex flex-col gap-3">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <LuFingerprint className="size-4" />
          {t("profileInfo.sections.fingerprint")}
        </div>
        <p className="text-xs text-muted-foreground">
          {t("profileInfo.fingerprint.notSupported")}
        </p>
      </div>
    );
  }

  // Viewing and editing fingerprints both require an active paid plan
  // (`crossOsUnlocked` is that paid flag here). Render a locked state instead of
  // the editor so free users can neither see nor change the fingerprint.
  if (!crossOsUnlocked) {
    return (
      <div className="flex flex-col items-center gap-3 rounded-lg border p-6 text-center">
        <LuLock className="size-4 shrink-0 text-muted-foreground" />
        <h3 className="text-sm font-medium text-foreground">
          {t("profileInfo.fingerprint.lockedTitle")}
        </h3>
        <p className="max-w-[48ch] text-sm text-pretty text-muted-foreground">
          {t("profileInfo.fingerprint.lockedDescription")}
        </p>
      </div>
    );
  }

  const onCamoufoxChange = (key: keyof CamoufoxConfig, value: unknown) => {
    setCamoufoxConfig((prev) => ({ ...prev, [key]: value }));
    setSuccess(null);
  };
  const onWayfernChange = (key: keyof WayfernConfig, value: unknown) => {
    setWayfernConfig((prev) => ({ ...prev, [key]: value }));
    setSuccess(null);
  };

  const onSave = async () => {
    setIsSaving(true);
    setError(null);
    setSuccess(null);
    try {
      if (isCamoufox) {
        await invoke("update_camoufox_config", {
          profileId: profile.id,
          config: camoufoxConfig,
        });
      } else {
        await invoke("update_wayfern_config", {
          profileId: profile.id,
          config: wayfernConfig,
        });
      }
      setSuccess(t("common.buttons.saved"));
      // Close the dialog once the fingerprint is saved.
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setIsSaving(false);
    }
  };

  const initial = isCamoufox
    ? JSON.stringify(profile.camoufox_config ?? {})
    : JSON.stringify(profile.wayfern_config ?? {});
  const current = isCamoufox
    ? JSON.stringify(camoufoxConfig)
    : JSON.stringify(wayfernConfig);
  const dirty = current !== initial;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 text-sm font-semibold">
        <LuFingerprint className="size-4" />
        {t("profileInfo.sections.fingerprint")}
      </div>
      <p className="text-xs text-muted-foreground">
        {t("profileInfo.sectionDesc.fingerprint")}
      </p>

      {isCamoufox && (
        <SharedCamoufoxConfigForm
          config={camoufoxConfig}
          onConfigChange={onCamoufoxChange}
          forceAdvanced={true}
          readOnly={isDisabled}
          browserType="camoufox"
          crossOsUnlocked={crossOsUnlocked}
          limitedMode={false}
          profileVersion={profile.version}
          profileBrowser={profile.browser}
        />
      )}
      {isWayfern && (
        <WayfernConfigForm
          config={wayfernConfig}
          onConfigChange={onWayfernChange}
          forceAdvanced={true}
          readOnly={isDisabled}
          crossOsUnlocked={crossOsUnlocked}
          profileVersion={profile.version}
          profileBrowser={profile.browser}
        />
      )}

      {error && <p className="text-xs text-destructive">{error}</p>}
      {success && !error && <p className="text-xs text-success">{success}</p>}

      <div className="flex items-center gap-2 mt-3 pt-3 border-t border-border">
        <Button
          size="sm"
          className="h-7 text-xs"
          disabled={!dirty || isSaving || isDisabled}
          onClick={() => {
            void onSave();
          }}
        >
          {isSaving ? t("common.buttons.saving") : t("common.buttons.save")}
        </Button>
        {dirty && (
          <Button
            size="sm"
            variant="ghost"
            className="h-7 text-xs"
            onClick={() => {
              setCamoufoxConfig(profile.camoufox_config ?? {});
              setWayfernConfig(profile.wayfern_config ?? {});
              setError(null);
              setSuccess(null);
            }}
          >
            {t("common.buttons.cancel")}
          </Button>
        )}
      </div>
    </div>
  );
}

function SecuritySectionInline({
  profile,
  isRunning,
  t,
}: {
  profile: BrowserProfile;
  isRunning: boolean;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  // Mode is implied by current state: unprotected → "set"; protected →
  // "change" by default, with a "remove" alternative.
  type Mode = "set" | "change" | "remove";
  const initialMode: Mode = profile.password_protected ? "change" : "set";
  const [mode, setMode] = React.useState<Mode>(initialMode);
  const [oldPassword, setOldPassword] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [confirm, setConfirm] = React.useState("");
  const [isSubmitting, setIsSubmitting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [success, setSuccess] = React.useState<string | null>(null);
  const [isVerifyOpen, setIsVerifyOpen] = React.useState(false);
  const [verifyPassword, setVerifyPassword] = React.useState("");
  const [isVerifying, setIsVerifying] = React.useState(false);

  const onVerify = async () => {
    setIsVerifying(true);
    try {
      await invoke("verify_profile_password", {
        profileId: profile.id,
        password: verifyPassword,
      });
      showSuccessToast(t("profilePassword.verifyDialog.matchToast"));
      setIsVerifyOpen(false);
      setVerifyPassword("");
    } catch (e) {
      const message = translateBackendError(
        t as unknown as Parameters<typeof translateBackendError>[0],
        e,
      );
      showErrorToast(message);
    } finally {
      setIsVerifying(false);
    }
  };

  // Reset the form whenever the underlying profile state changes (e.g. the
  // user just set a password — flip to "change" mode and clear fields).
  React.useEffect(() => {
    setMode(profile.password_protected ? "change" : "set");
    setOldPassword("");
    setPassword("");
    setConfirm("");
    setError(null);
    setSuccess(null);
  }, [profile.password_protected]);

  const reset = () => {
    setOldPassword("");
    setPassword("");
    setConfirm("");
    setError(null);
  };

  const validate = (): string | null => {
    if (mode === "change" || mode === "remove") {
      if (!oldPassword) return t("profilePassword.errors.passwordRequired");
    }
    if (mode === "set" || mode === "change") {
      if (password.length < 8) return t("profilePassword.errors.tooShort");
      if (password !== confirm)
        return t("profilePassword.errors.passwordMismatch");
    }
    return null;
  };

  const onSubmit = async () => {
    if (isRunning) return;
    const v = validate();
    if (v) {
      setError(v);
      return;
    }
    setIsSubmitting(true);
    setError(null);
    setSuccess(null);
    try {
      if (mode === "set") {
        await invoke("set_profile_password", {
          profileId: profile.id,
          password,
        });
        showSuccessToast(t("profilePassword.toasts.set"));
      } else if (mode === "change") {
        await invoke("change_profile_password", {
          profileId: profile.id,
          oldPassword,
          newPassword: password,
        });
        showSuccessToast(t("profilePassword.toasts.changed"));
      } else {
        await invoke("remove_profile_password", {
          profileId: profile.id,
          password: oldPassword,
        });
        showSuccessToast(t("profilePassword.toasts.removed"));
      }
      reset();
    } catch (e) {
      const message = translateBackendError(
        t as unknown as Parameters<typeof translateBackendError>[0],
        e,
      );
      setError(message);
      showErrorToast(message);
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 text-sm font-semibold">
        <LuKey className="size-4" />
        {t("profileInfo.sections.security")}
      </div>
      <p className="text-xs text-muted-foreground">
        {profile.password_protected
          ? t("profileInfo.security.protected")
          : t("profileInfo.security.unprotected")}
      </p>

      {profile.password_protected && (
        <div className="flex gap-1.5">
          <button
            type="button"
            onClick={() => {
              setVerifyPassword("");
              setIsVerifyOpen(true);
            }}
            className={cn(
              "flex-1 h-7 px-2 text-xs rounded-md border transition-colors",
              "border-border text-muted-foreground hover:text-foreground hover:bg-accent/50",
            )}
          >
            {t("profilePassword.modes.validate")}
          </button>
          <button
            type="button"
            onClick={() => {
              setMode("change");
              reset();
            }}
            className={cn(
              "flex-1 h-7 px-2 text-xs rounded-md border transition-colors",
              mode === "change"
                ? "bg-accent text-accent-foreground border-transparent"
                : "border-border text-muted-foreground hover:text-foreground hover:bg-accent/50",
            )}
          >
            {t("profilePassword.modes.change")}
          </button>
          <button
            type="button"
            onClick={() => {
              setMode("remove");
              reset();
            }}
            className={cn(
              "flex-1 h-7 px-2 text-xs rounded-md border transition-colors",
              mode === "remove"
                ? "bg-destructive/10 text-destructive border-transparent"
                : "border-border text-muted-foreground hover:text-foreground hover:bg-accent/50",
            )}
          >
            {t("profilePassword.modes.remove")}
          </button>
        </div>
      )}

      <div className="flex flex-col gap-2">
        {(mode === "change" || mode === "remove") && (
          <Input
            type="password"
            value={oldPassword}
            onChange={(e) => {
              setOldPassword(e.target.value);
              setError(null);
            }}
            placeholder={t("profilePassword.fields.currentPassword")}
            disabled={isRunning || isSubmitting}
            className="h-8 text-xs"
          />
        )}
        {(mode === "set" || mode === "change") && (
          <>
            <Input
              type="password"
              value={password}
              onChange={(e) => {
                setPassword(e.target.value);
                setError(null);
              }}
              placeholder={t("profilePassword.fields.newPassword")}
              disabled={isRunning || isSubmitting}
              className="h-8 text-xs"
            />
            <Input
              type="password"
              value={confirm}
              onChange={(e) => {
                setConfirm(e.target.value);
                setError(null);
              }}
              placeholder={t("profilePassword.fields.confirmPassword")}
              disabled={isRunning || isSubmitting}
              className="h-8 text-xs"
            />
          </>
        )}
      </div>

      {error && <p className="text-xs text-destructive">{error}</p>}
      {success && !error && <p className="text-xs text-success">{success}</p>}

      {isRunning && (
        <p className="text-xs text-muted-foreground">
          {t("profileInfo.security.cannotWhileRunning")}
        </p>
      )}

      <Button
        size="sm"
        variant={mode === "remove" ? "destructive" : "default"}
        className="self-start h-7 text-xs"
        disabled={isRunning || isSubmitting}
        onClick={() => {
          void onSubmit();
        }}
      >
        {mode === "set"
          ? t("profilePassword.modes.set")
          : mode === "change"
            ? t("profilePassword.modes.change")
            : t("profilePassword.modes.remove")}
      </Button>

      <Dialog
        open={isVerifyOpen}
        onOpenChange={(open) => {
          if (!isVerifying) {
            setIsVerifyOpen(open);
            if (!open) setVerifyPassword("");
          }
        }}
      >
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("profilePassword.verifyDialog.title")}</DialogTitle>
            <DialogDescription>
              {t("profilePassword.verifyDialog.description")}
            </DialogDescription>
          </DialogHeader>
          <Input
            type="password"
            placeholder={t("profilePassword.fields.currentPassword")}
            value={verifyPassword}
            autoFocus
            onChange={(e) => setVerifyPassword(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && verifyPassword.length > 0) {
                e.preventDefault();
                void onVerify();
              }
            }}
          />
          <DialogFooter>
            <Button
              variant="outline"
              disabled={isVerifying}
              onClick={() => {
                setIsVerifyOpen(false);
                setVerifyPassword("");
              }}
            >
              {t("common.buttons.cancel")}
            </Button>
            <Button
              disabled={isVerifying || verifyPassword.length === 0}
              onClick={() => void onVerify()}
            >
              {isVerifying
                ? t("common.buttons.loading")
                : t("profilePassword.verifyDialog.submit")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

interface ProfileLaunchHookDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profileId: string | null;
  currentLaunchHook: string | null;
}

export function ProfileLaunchHookDialog({
  isOpen,
  onClose,
  profileId,
  currentLaunchHook,
}: ProfileLaunchHookDialogProps) {
  const { t } = useTranslation();
  const [value, setValue] = React.useState(currentLaunchHook ?? "");
  const [isSaving, setIsSaving] = React.useState(false);

  React.useEffect(() => {
    if (isOpen) {
      setValue(currentLaunchHook ?? "");
    }
  }, [isOpen, currentLaunchHook]);

  const trimmed = value.trim();
  const saved = currentLaunchHook ?? "";
  const isDirty = trimmed !== saved;

  const handleSave = async () => {
    if (!profileId) return;
    setIsSaving(true);
    try {
      await invoke("update_profile_launch_hook", {
        profileId,
        launchHook: trimmed || null,
      });
      onClose();
    } catch (err) {
      console.error("Failed to update launch hook:", err);
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("profileInfo.launchHook.title")}</DialogTitle>
        </DialogHeader>
        <p className="text-xs text-muted-foreground">
          {t("profileInfo.launchHook.description")}
        </p>
        <Input
          value={value}
          onChange={(e) => {
            setValue(e.target.value);
          }}
          placeholder={t("profileInfo.launchHook.placeholder")}
          disabled={isSaving}
        />
        <DialogFooter>
          <Button
            onClick={() => void handleSave()}
            disabled={isSaving || !isDirty}
            className="w-full"
          >
            {t("common.buttons.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface ProfileDnsBlocklistDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profileId: string | null;
  currentLevel: string | null;
}

export function ProfileDnsBlocklistDialog({
  isOpen,
  onClose,
  profileId,
  currentLevel,
}: ProfileDnsBlocklistDialogProps) {
  const { t } = useTranslation();
  const [level, setLevel] = React.useState(currentLevel ?? "");
  const [isSaving, setIsSaving] = React.useState(false);

  React.useEffect(() => {
    if (isOpen) {
      setLevel(currentLevel ?? "");
    }
  }, [isOpen, currentLevel]);

  const handleSave = async () => {
    if (!profileId) return;
    setIsSaving(true);
    try {
      await invoke("update_profile_dns_blocklist", {
        profileId,
        dnsBlocklist: level || null,
      });
      onClose();
    } catch (err) {
      console.error("Failed to update DNS blocklist:", err);
    } finally {
      setIsSaving(false);
    }
  };

  const options = [
    { value: "", label: t("dnsBlocklist.none") },
    { value: "light", label: t("dnsBlocklist.light") },
    { value: "normal", label: t("dnsBlocklist.normal") },
    { value: "pro", label: t("dnsBlocklist.pro") },
    { value: "pro_plus", label: t("dnsBlocklist.proPlus") },
    { value: "ultimate", label: t("dnsBlocklist.ultimate") },
  ];

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-xs">
        <DialogHeader>
          <DialogTitle>{t("dnsBlocklist.title")}</DialogTitle>
        </DialogHeader>
        <p className="text-xs text-muted-foreground">
          {t("dnsBlocklist.settingsDescription")}{" "}
          <a
            href="https://github.com/hagezi/dns-blocklists"
            target="_blank"
            rel="noopener noreferrer"
            className="text-primary hover:underline"
          >
            {t("common.buttons.moreInfo")}
          </a>
        </p>
        <div className="space-y-1">
          {options.map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => setLevel(option.value)}
              className={`w-full text-left px-3 py-2 rounded-md text-sm transition-colors ${
                level === option.value
                  ? "bg-primary/10 text-primary border border-primary/30"
                  : "hover:bg-accent border border-transparent"
              }`}
            >
              {option.label}
            </button>
          ))}
        </div>
        <Button
          onClick={() => void handleSave()}
          disabled={isSaving || level === (currentLevel ?? "")}
          className="w-full"
        >
          {t("common.buttons.save")}
        </Button>
      </DialogContent>
    </Dialog>
  );
}

interface ProfileBypassRulesDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profileId: string | null;
  initialRules?: string[];
}

export function ProfileBypassRulesDialog({
  isOpen,
  onClose,
  profileId,
  initialRules,
}: ProfileBypassRulesDialogProps) {
  const { t } = useTranslation();
  const [bypassRules, setBypassRules] = React.useState<string[]>([]);
  const [newRule, setNewRule] = React.useState("");

  React.useEffect(() => {
    if (isOpen) {
      setBypassRules(initialRules ?? []);
      setNewRule("");
    }
  }, [isOpen, initialRules]);

  const updateBypassRules = async (rules: string[]) => {
    if (!profileId) return;
    try {
      await invoke("update_profile_proxy_bypass_rules", {
        profileId,
        rules,
      });
      setBypassRules(rules);
    } catch {
      // ignore
    }
  };

  const handleAddRule = () => {
    const trimmed = newRule.trim();
    if (!trimmed || bypassRules.includes(trimmed)) return;
    const updated = [...bypassRules, trimmed];
    setNewRule("");
    void updateBypassRules(updated);
  };

  const handleRemoveRule = (rule: string) => {
    void updateBypassRules(bypassRules.filter((r) => r !== rule));
  };

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="sm:max-w-lg max-h-[80vh] flex flex-col">
        <DialogHeader className="shrink-0">
          <DialogTitle>{t("profileInfo.network.bypassRulesTitle")}</DialogTitle>
        </DialogHeader>
        <ScrollArea className="flex-1 min-h-0">
          <div className="flex flex-col gap-3 py-2">
            <p className="text-sm text-muted-foreground">
              {t("profileInfo.network.bypassRulesDescription")}
            </p>
            <div className="flex gap-2">
              <Input
                value={newRule}
                onChange={(e) => {
                  setNewRule(e.target.value);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleAddRule();
                }}
                placeholder={t("profileInfo.network.rulePlaceholder")}
                className="flex-1 text-sm"
              />
              <Button
                size="sm"
                onClick={handleAddRule}
                disabled={!newRule.trim()}
              >
                <LuPlus className="size-4 mr-1" />
                {t("profileInfo.network.addRule")}
              </Button>
            </div>
            {bypassRules.length === 0 ? (
              <p className="text-sm text-muted-foreground py-2">
                {t("profileInfo.network.noRules")}
              </p>
            ) : (
              <div className="flex flex-col gap-1.5">
                {bypassRules.map((rule) => (
                  <div
                    key={rule}
                    className="flex items-center justify-between gap-2 px-3 py-1.5 rounded-md bg-muted text-sm"
                  >
                    <span className="font-mono text-xs truncate">{rule}</span>
                    <button
                      type="button"
                      onClick={() => {
                        handleRemoveRule(rule);
                      }}
                      className="text-muted-foreground hover:text-destructive transition-colors shrink-0"
                    >
                      <LuX className="size-3.5" />
                    </button>
                  </div>
                ))}
              </div>
            )}
            <p className="text-xs text-muted-foreground">
              {t("profileInfo.network.ruleTypes")}
            </p>
          </div>
        </ScrollArea>
        <DialogFooter className="shrink-0">
          <Button variant="outline" onClick={onClose}>
            {t("common.buttons.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
