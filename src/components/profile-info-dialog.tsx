"use client";

import { invoke } from "@tauri-apps/api/core";
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
  LuPlus,
  LuPuzzle,
  LuRefreshCw,
  LuSettings,
  LuShieldCheck,
  LuTrash2,
  LuX,
} from "react-icons/lu";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ProBadge } from "@/components/ui/pro-badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  getBrowserDisplayName,
  getOSDisplayName,
  getProfileIcon,
  isCrossOsProfile,
} from "@/lib/browser-utils";
import { formatRelativeTime } from "@/lib/flag-utils";
import { cn } from "@/lib/utils";
import type {
  BrowserProfile,
  ProfileGroup,
  StoredProxy,
  VpnConfig,
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
  onCloneProfile?: (profile: BrowserProfile) => void;
  onDeleteProfile?: (profile: BrowserProfile) => void;
  crossOsUnlocked?: boolean;
  isRunning?: boolean;
  isDisabled?: boolean;
  isCrossOs?: boolean;
  syncStatuses: Record<string, { status: string; error?: string }>;
}

function OSIcon({ os }: { os: string }) {
  switch (os) {
    case "macos":
      return <FaApple className="w-3.5 h-3.5" />;
    case "windows":
      return <FaWindows className="w-3.5 h-3.5" />;
    case "linux":
      return <FaLinux className="w-3.5 h-3.5" />;
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
  onCloneProfile,
  onDeleteProfile,
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
    (async () => {
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
    (async () => {
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
  const syncLabel = syncStatus
    ? `${syncMode} (${syncStatus.status})`
    : syncMode;

  const handleCopyId = async () => {
    try {
      await navigator.clipboard.writeText(profile.id);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // ignore
    }
  };

  const handleAction = (action: () => void) => {
    onClose();
    action();
  };

  const releaseLabel =
    profile.release_type.charAt(0).toUpperCase() +
    profile.release_type.slice(1);
  const hasTags = profile.tags && profile.tags.length > 0;
  const hasNote = !!profile.note;
  const showCrossOs = !!(profile.host_os && isCrossOsProfile(profile));

  type ActionItem = {
    icon: React.ReactNode;
    label: string;
    onClick: () => void;
    disabled?: boolean;
    destructive?: boolean;
    proBadge?: boolean;
    runningBadge?: boolean;
    hidden?: boolean;
  };

  const actions: ActionItem[] = [
    {
      icon: <LuGlobe className="w-4 h-4" />,
      label: t("profiles.actions.viewNetwork"),
      onClick: () => handleAction(() => onOpenTrafficDialog?.(profile.id)),
      disabled: isCrossOs,
    },
    {
      icon: <LuRefreshCw className="w-4 h-4" />,
      label: t("profiles.actions.syncSettings"),
      onClick: () => handleAction(() => onOpenProfileSyncDialog?.(profile)),
      disabled: isCrossOs,
      hidden: profile.ephemeral === true,
    },
    {
      icon: <LuGroup className="w-4 h-4" />,
      label: t("profiles.actions.assignToGroup"),
      onClick: () =>
        handleAction(() => onAssignProfilesToGroup?.([profile.id])),
      disabled: isDisabled,
      runningBadge: isRunning,
    },
    {
      icon: <LuFingerprint className="w-4 h-4" />,
      label: t("profiles.actions.changeFingerprint"),
      onClick: () => handleAction(() => onConfigureCamoufox?.(profile)),
      disabled: isDisabled,
      runningBadge: isRunning,
      hidden: !isCamoufoxOrWayfern || !onConfigureCamoufox,
    },
    {
      icon: <LuCopy className="w-4 h-4" />,
      label: t("profiles.actions.copyCookiesToProfile"),
      onClick: () => handleAction(() => onCopyCookiesToProfile?.(profile)),
      disabled: isDisabled || !crossOsUnlocked,
      proBadge: !crossOsUnlocked,
      runningBadge: isRunning && crossOsUnlocked,
      hidden:
        !isCamoufoxOrWayfern ||
        profile.ephemeral === true ||
        !onCopyCookiesToProfile,
    },
    {
      icon: <LuCookie className="w-4 h-4" />,
      label: t("profileInfo.actions.manageCookies"),
      onClick: () => handleAction(() => onOpenCookieManagement?.(profile)),
      disabled: isDisabled || !crossOsUnlocked,
      proBadge: !crossOsUnlocked,
      runningBadge: isRunning && crossOsUnlocked,
      hidden:
        !isCamoufoxOrWayfern ||
        profile.ephemeral === true ||
        !onOpenCookieManagement,
    },
    {
      icon: <LuSettings className="w-4 h-4" />,
      label: t("profiles.actions.clone"),
      onClick: () => handleAction(() => onCloneProfile?.(profile)),
      disabled: isDisabled,
      runningBadge: isRunning,
      hidden: profile.ephemeral === true,
    },
    {
      icon: <LuPuzzle className="w-4 h-4" />,
      label: t("profileInfo.actions.assignExtensionGroup"),
      onClick: () => handleAction(() => onAssignExtensionGroup?.([profile.id])),
      disabled: isDisabled || !crossOsUnlocked,
      proBadge: !crossOsUnlocked,
      runningBadge: isRunning && crossOsUnlocked,
      hidden: profile.ephemeral === true,
    },
    {
      icon: <LuShieldCheck className="w-4 h-4" />,
      label: t("profileInfo.network.bypassRulesTitle"),
      onClick: () => handleAction(() => onOpenBypassRules?.(profile)),
    },
    {
      icon: <LuTrash2 className="w-4 h-4" />,
      label: t("profiles.actions.delete"),
      onClick: () => handleAction(() => onDeleteProfile?.(profile)),
      disabled: isDeleteDisabled,
      destructive: true,
    },
  ];

  const visibleActions = actions.filter((a) => !a.hidden);

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("profileInfo.title")}</DialogTitle>
        </DialogHeader>
        <Tabs defaultValue="info">
          <TabsList className="w-full">
            <TabsTrigger value="info" className="flex-1">
              {t("profileInfo.tabs.info")}
            </TabsTrigger>
            <TabsTrigger value="settings" className="flex-1">
              {t("profileInfo.tabs.settings")}
            </TabsTrigger>
          </TabsList>
          <TabsContent value="info">
            <div className="overflow-y-auto max-h-[calc(80vh-12rem)] pr-1">
              <div className="flex flex-col gap-4 py-3">
                {/* Hero */}
                <div className="flex items-center gap-3">
                  <div className="rounded-lg bg-muted p-2.5 shrink-0">
                    <ProfileIcon className="w-8 h-8 text-foreground" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <h3 className="text-base font-semibold truncate">
                      {profile.name}
                    </h3>
                    <div className="flex flex-wrap items-center gap-1.5 mt-1">
                      <Badge variant="secondary" className="text-xs">
                        {getBrowserDisplayName(profile.browser)}{" "}
                        {profile.version}
                      </Badge>
                      <Badge variant="outline" className="text-xs">
                        {releaseLabel}
                      </Badge>
                      {isRunning && (
                        <Badge className="text-xs bg-primary/15 text-primary border-primary/25">
                          {t("common.status.running")}
                        </Badge>
                      )}
                      {profile.ephemeral && (
                        <Badge variant="outline" className="text-xs">
                          {t("profiles.ephemeralBadge")}
                        </Badge>
                      )}
                      {showCrossOs && profile.host_os && (
                        <Badge variant="outline" className="text-xs gap-1">
                          <OSIcon os={profile.host_os} />
                          {getOSDisplayName(profile.host_os)}
                        </Badge>
                      )}
                    </div>
                  </div>
                </div>

                {/* Profile ID */}
                <div className="flex items-center gap-2 rounded-md bg-muted/50 border px-3 py-2">
                  <span className="text-xs text-muted-foreground shrink-0">
                    ID
                  </span>
                  <span className="font-mono text-xs truncate flex-1">
                    {profile.id}
                  </span>
                  <button
                    type="button"
                    onClick={() => void handleCopyId()}
                    className="text-muted-foreground hover:text-foreground transition-colors shrink-0"
                  >
                    {copied ? (
                      <LuClipboardCheck className="w-3.5 h-3.5" />
                    ) : (
                      <LuClipboard className="w-3.5 h-3.5" />
                    )}
                  </button>
                </div>

                {/* Network & Organization */}
                <div className="grid grid-cols-2 gap-2">
                  <InfoCard
                    label={t("profileInfo.fields.proxyVpn")}
                    value={networkLabel}
                  />
                  <InfoCard
                    label={t("profileInfo.fields.group")}
                    value={groupName ?? t("profileInfo.values.none")}
                  />
                  <InfoCard
                    label={t("profileInfo.fields.extensionGroup")}
                    value={extensionGroupName ?? t("profileInfo.values.none")}
                  />
                  <InfoCard
                    label={t("profileInfo.fields.lastLaunched")}
                    value={
                      profile.last_launch
                        ? formatRelativeTime(profile.last_launch)
                        : t("profileInfo.values.never")
                    }
                  />
                </div>

                {/* Sync */}
                <div className="rounded-md bg-muted/50 border px-3 py-2.5 flex items-center justify-between">
                  <div>
                    <p className="text-xs text-muted-foreground">
                      {t("profileInfo.fields.syncStatus")}
                    </p>
                    <p className="text-sm mt-0.5">{syncLabel}</p>
                  </div>
                  <Badge
                    variant={syncMode === "Disabled" ? "outline" : "secondary"}
                    className="text-xs shrink-0"
                  >
                    {syncMode === "Disabled"
                      ? t("sync.mode.disabled")
                      : syncStatus?.status === "syncing"
                        ? t("common.status.syncing")
                        : t("common.status.synced")}
                  </Badge>
                </div>

                {/* Tags */}
                {hasTags && (
                  <div className="flex flex-col gap-1.5">
                    <span className="text-xs text-muted-foreground">
                      {t("profileInfo.fields.tags")}
                    </span>
                    <div className="flex flex-wrap gap-1.5">
                      {profile.tags?.map((tag) => (
                        <Badge
                          key={tag}
                          variant="secondary"
                          className="text-xs"
                        >
                          {tag}
                        </Badge>
                      ))}
                    </div>
                  </div>
                )}

                {/* Note */}
                {hasNote && (
                  <div className="flex flex-col gap-1.5">
                    <span className="text-xs text-muted-foreground">
                      {t("profileInfo.fields.note")}
                    </span>
                    <p className="text-sm rounded-md bg-muted/50 border px-3 py-2 whitespace-pre-wrap break-words">
                      {profile.note}
                    </p>
                  </div>
                )}

                {/* Team */}
                {profile.created_by_email && (
                  <div className="rounded-md bg-muted/50 border px-3 py-2.5">
                    <p className="text-xs text-muted-foreground">
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
            </div>
          </TabsContent>
          <TabsContent value="settings">
            <div className="overflow-y-auto max-h-[calc(80vh-12rem)]">
              <div className="flex flex-col py-1">
                {visibleActions.map((action) => (
                  <button
                    key={action.label}
                    type="button"
                    disabled={action.disabled}
                    onClick={action.onClick}
                    className={cn(
                      "flex items-center gap-3 px-3 py-2.5 rounded-md text-sm transition-colors text-left w-full",
                      "hover:bg-accent disabled:opacity-50 disabled:pointer-events-none",
                      action.destructive &&
                        "text-destructive hover:bg-destructive/10",
                    )}
                  >
                    {action.icon}
                    <span className="flex-1 flex items-center gap-2">
                      {action.label}
                      {action.runningBadge && (
                        <span className="px-1.5 py-0.5 text-[10px] font-semibold rounded bg-primary/15 text-primary uppercase">
                          {t("common.status.running")}
                        </span>
                      )}
                      {action.proBadge && !action.runningBadge && <ProBadge />}
                    </span>
                    <LuChevronRight className="w-4 h-4 text-muted-foreground" />
                  </button>
                ))}
              </div>
            </div>
          </TabsContent>
        </Tabs>
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
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
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
                onChange={(e) => setNewRule(e.target.value)}
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
                <LuPlus className="w-4 h-4 mr-1" />
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
                      onClick={() => handleRemoveRule(rule)}
                      className="text-muted-foreground hover:text-destructive transition-colors shrink-0"
                    >
                      <LuX className="w-3.5 h-3.5" />
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
