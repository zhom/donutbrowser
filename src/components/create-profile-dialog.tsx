"use client";

import * as CheckboxPrimitive from "@radix-ui/react-checkbox";
import { invoke } from "@tauri-apps/api/core";
import { motion, useReducedMotion } from "motion/react";
import type * as React from "react";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import {
  LuCheck,
  LuChevronDown,
  LuChevronRight,
  LuFlame,
  LuGlobe,
  LuLoaderCircle,
  LuLock,
  LuLockOpen,
  LuShieldCheck,
  LuTriangleAlert,
} from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import { ProfileGlyph } from "@/components/profile-glyph";
import { ProxyFormDialog } from "@/components/proxy-form-dialog";
import {
  AnimatedDisclosureChevron,
  AnimatedDisclosureContent,
} from "@/components/ui/animated-disclosure";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { SoftFields } from "@/components/ui/field-variant";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { ProBadge } from "@/components/ui/pro-badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { WayfernConfigForm } from "@/components/wayfern-config-form";
import { useBrowserDownload } from "@/hooks/use-browser-download";
import { useInputModality } from "@/hooks/use-input-modality";
import { useProxyEvents } from "@/hooks/use-proxy-events";
import { useVpnEvents } from "@/hooks/use-vpn-events";
import { getOSDisplayName, getOSIcon } from "@/lib/browser-utils";
import { DNS_BLOCKLIST_LEVELS } from "@/lib/dns-blocklist-levels";
import { MOTION_EASE_OUT } from "@/lib/motion";
import { cn } from "@/lib/utils";
import type { BrowserReleaseTypes, WayfernConfig, WayfernOS } from "@/types";

const PASSWORD_MIN_LEN = 8;

const OS_OPTIONS: WayfernOS[] = ["windows", "macos", "linux", "android", "ios"];

const getCurrentOS = (): WayfernOS => {
  if (typeof navigator === "undefined") return "linux";
  const platform = navigator.platform.toLowerCase();
  if (platform.includes("win")) return "windows";
  if (platform.includes("mac")) return "macos";
  return "linux";
};

const defaultWayfernConfig = (): WayfernConfig => ({
  os: getCurrentOS(),
  screen_max_width:
    typeof window === "undefined" ? undefined : window.screen.width,
  screen_max_height:
    typeof window === "undefined" ? undefined : window.screen.height,
});

interface CreateProfileDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onCreateProfile: (profileData: {
    name: string;
    browserStr: "wayfern";
    version: string;
    releaseType: string;
    proxyId?: string;
    vpnId?: string;
    wayfernConfig?: WayfernConfig;
    groupId?: string;
    extensionGroupId?: string;
    ephemeral?: boolean;
    dnsBlocklist?: string;
    launchHook?: string;
    password?: string;
  }) => Promise<void>;
  selectedGroupId?: string;
  crossOsUnlocked?: boolean;
}

export function CreateProfileDialog({
  isOpen,
  onClose,
  onCreateProfile,
  selectedGroupId,
  crossOsUnlocked = false,
}: CreateProfileDialogProps) {
  const { t } = useTranslation();
  const proxyListboxId = useId();
  const proxyLabelId = useId();
  const proxyValueId = useId();
  const osListboxId = useId();
  const osLabelId = useId();
  const osValueId = useId();
  const advancedOptionsId = useId();
  const reduceMotion = useReducedMotion() ?? false;
  const inputModality = useInputModality();
  const [profileName, setProfileName] = useState("");
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [selectedProxyId, setSelectedProxyId] = useState<string>();
  const [proxyPopoverOpen, setProxyPopoverOpen] = useState(false);
  const [osPopoverOpen, setOsPopoverOpen] = useState(false);
  const [showProxyForm, setShowProxyForm] = useState(false);
  const [dnsBlocklist, setDnsBlocklist] = useState("");
  const [launchHook, setLaunchHook] = useState("");
  const [wayfernConfig, setWayfernConfig] =
    useState<WayfernConfig>(defaultWayfernConfig);
  const [isCreating, setIsCreating] = useState(false);
  const [ephemeral, setEphemeral] = useState(false);
  const [allowance, setAllowance] = useState<ProfileCreationAllowance | null>(
    null,
  );

  const loadAllowance = useCallback(async () => {
    try {
      setAllowance(
        await invoke<ProfileCreationAllowance>(
          "get_profile_creation_allowance",
        ),
      );
    } catch (error) {
      console.error("Failed to read the profile creation allowance:", error);
    }
  }, []);

  useEffect(() => {
    if (isOpen) void loadAllowance();
  }, [isOpen, loadAllowance]);

  // The window frees a slot by itself, so look again once it should have.
  useEffect(() => {
    const wait = allowance?.retry_after_secs;
    if (!isOpen || wait == null) return;
    const timer = window.setTimeout(
      () => void loadAllowance(),
      wait * 1000 + 500,
    );
    return () => window.clearTimeout(timer);
  }, [isOpen, allowance, loadAllowance]);

  const overAllowance =
    allowance !== null && allowance.used >= allowance.per_hour;
  const hourlyCapReached = overAllowance && allowance.enforcement === "hard";
  const creatingQuickly = overAllowance && allowance.enforcement === "soft";
  const [enablePassword, setEnablePassword] = useState(false);
  const [password, setPassword] = useState("");
  const [passwordConfirm, setPasswordConfirm] = useState("");
  const [passwordError, setPasswordError] = useState<string | null>(null);
  const [selectedExtensionGroupId, setSelectedExtensionGroupId] =
    useState<string>();
  const [extensionGroups, setExtensionGroups] = useState<
    { id: string; name: string; extension_ids: string[] }[]
  >([]);
  const [releaseTypes, setReleaseTypes] = useState<BrowserReleaseTypes>({});
  const [isLoadingReleaseTypes, setIsLoadingReleaseTypes] = useState(false);
  const [releaseTypesError, setReleaseTypesError] = useState(false);
  const versionRequestRef = useRef(0);
  const { storedProxies } = useProxyEvents();
  const { vpnConfigs } = useVpnEvents();
  const {
    isBrowserDownloading,
    downloadBrowser,
    loadDownloadedVersions,
    downloadedVersionsMap,
  } = useBrowserDownload();

  const loadReleaseTypes = useCallback(async () => {
    const request = ++versionRequestRef.current;
    setIsLoadingReleaseTypes(true);
    setReleaseTypesError(false);
    try {
      const releases = await invoke<BrowserReleaseTypes>(
        "get_browser_release_types",
        {
          browserStr: "wayfern",
        },
      );
      await loadDownloadedVersions("wayfern");
      if (versionRequestRef.current === request) {
        setReleaseTypes(releases.stable ? { stable: releases.stable } : {});
      }
    } catch (error) {
      console.error("Failed to load Wayfern release types:", error);
      try {
        const downloaded = await loadDownloadedVersions("wayfern");
        if (versionRequestRef.current === request) {
          setReleaseTypes(downloaded.length ? { stable: downloaded[0] } : {});
          setReleaseTypesError(downloaded.length === 0);
        }
      } catch (error) {
        console.error("Failed to load downloaded Wayfern versions:", error);
        if (versionRequestRef.current === request) setReleaseTypesError(true);
      }
    } finally {
      if (versionRequestRef.current === request)
        setIsLoadingReleaseTypes(false);
    }
  }, [loadDownloadedVersions]);

  useEffect(() => {
    if (!isOpen) return;
    void loadReleaseTypes();
    void invoke<{ id: string; name: string; extension_ids: string[] }[]>(
      "list_extension_groups",
    )
      .then(setExtensionGroups)
      .catch(() => setExtensionGroups([]));
    void invoke<boolean>("is_geoip_database_available")
      .then(async (available) => {
        if (!available) await invoke("download_geoip_database");
      })
      .catch((error) =>
        console.error("Failed to prepare GeoIP database:", error),
      );
    return () => {
      versionRequestRef.current += 1;
    };
  }, [isOpen, loadReleaseTypes]);

  const downloadedVersions = downloadedVersionsMap.wayfern ?? [];
  const latestVersion = releaseTypes.stable;
  const creatableVersion =
    latestVersion && downloadedVersions.includes(latestVersion)
      ? latestVersion
      : downloadedVersions[0];
  const downloading = isBrowserDownloading("wayfern");
  const isCreateDisabled =
    !profileName.trim() ||
    !creatableVersion ||
    downloading ||
    isCreating ||
    hourlyCapReached;
  const selectedVpn = selectedProxyId?.startsWith("vpn-")
    ? vpnConfigs.find((vpn) => vpn.id === selectedProxyId.slice(4))
    : undefined;
  const selectedProxy = storedProxies.find(
    (proxy) => proxy.id === selectedProxyId,
  );
  const proxyLabel = selectedVpn
    ? `WG — ${selectedVpn.name}`
    : (selectedProxy?.name ?? t("createProfile.proxy.noProxy"));
  const hostOs = getCurrentOS();
  const selectedOs = wayfernConfig.os ?? hostOs;
  const crossOs = selectedOs !== hostOs;
  const SelectedOsIcon = getOSIcon(selectedOs);

  const clearPassword = () => {
    setEnablePassword(false);
    setPassword("");
    setPasswordConfirm("");
    setPasswordError(null);
  };

  const handleClose = () => {
    versionRequestRef.current += 1;
    setProfileName("");
    setAdvancedOpen(false);
    setSelectedProxyId(undefined);
    setProxyPopoverOpen(false);
    setOsPopoverOpen(false);
    setShowProxyForm(false);
    setDnsBlocklist("");
    setLaunchHook("");
    setSelectedExtensionGroupId(undefined);
    setReleaseTypes({});
    setIsLoadingReleaseTypes(false);
    setReleaseTypesError(false);
    setWayfernConfig(defaultWayfernConfig());
    setEphemeral(false);
    clearPassword();
    onClose();
  };

  const handleCreate = async () => {
    if (isCreateDisabled || !creatableVersion) return;
    if (enablePassword && !ephemeral) {
      if (password.length < PASSWORD_MIN_LEN) {
        setPasswordError(
          t("profilePassword.errors.tooShort", { min: PASSWORD_MIN_LEN }),
        );
        return;
      }
      if (password !== passwordConfirm) {
        setPasswordError(t("profilePassword.errors.mismatch"));
        return;
      }
    }
    setPasswordError(null);
    setIsCreating(true);
    const isVpnSelection = selectedProxyId?.startsWith("vpn-") ?? false;
    try {
      await onCreateProfile({
        name: profileName.trim(),
        browserStr: "wayfern",
        version: creatableVersion,
        releaseType: "stable",
        proxyId: isVpnSelection ? undefined : selectedProxyId,
        vpnId: isVpnSelection ? selectedProxyId?.slice(4) : undefined,
        wayfernConfig,
        groupId:
          selectedGroupId && selectedGroupId !== "__all__"
            ? selectedGroupId
            : undefined,
        extensionGroupId: selectedExtensionGroupId,
        ephemeral,
        dnsBlocklist: dnsBlocklist || undefined,
        launchHook: launchHook.trim() || undefined,
        password: enablePassword && !ephemeral ? password : undefined,
      });
      handleClose();
    } catch (error) {
      console.error("Failed to create profile:", error);
      void loadAllowance();
    } finally {
      setIsCreating(false);
    }
  };

  const handleDownload = async () => {
    if (!latestVersion) return;
    try {
      await downloadBrowser("wayfern", latestVersion);
    } catch (error) {
      console.error("Failed to download Wayfern:", error);
    }
  };

  const updateWayfernConfig = useCallback(
    (key: keyof WayfernConfig, value: unknown) => {
      setWayfernConfig((previous) => ({ ...previous, [key]: value }));
    },
    [],
  );

  const animateEntry = !reduceMotion && inputModality === "pointer";
  const rise = (delay: number) =>
    animateEntry
      ? {
          initial: { y: 6 },
          animate: { y: 0 },
          transition: { ...ENTRY_SPRING, delay },
        }
      : {};
  const createButton = (
    <LoadingButton
      onClick={handleCreate}
      isLoading={isCreating}
      disabled={isCreateDisabled}
      className="h-9 rounded-md px-4 transition-[opacity,box-shadow,transform] duration-200"
    >
      {t("common.buttons.create")}
    </LoadingButton>
  );

  const reveal = animateEntry
    ? {
        initial: { y: -6, scale: 0.985 },
        animate: { y: 0, scale: 1 },
        transition: ENTRY_SPRING,
      }
    : {};

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open && !isCreating) handleClose();
      }}
    >
      <DialogContent
        className="flex max-h-[calc(100dvh-max(1.5rem,12vh)-1.5rem)] max-w-lg flex-col gap-4 overflow-hidden rounded-2xl border-0 p-5 shadow-2xl ring-1 ring-foreground/10"
        // Anchored near the top rather than centered, so options that open
        // below push the footer down instead of moving the whole dialog.
        style={
          {
            top: "max(1.5rem, 12vh)",
            "--tw-translate-y": "0px",
          } as React.CSSProperties
        }
        aria-describedby={undefined}
        dismissible={!isCreating}
      >
        <DialogHeader className="shrink-0">
          <DialogTitle className="text-base">
            {t("createProfile.title")}
          </DialogTitle>
        </DialogHeader>

        <div
          data-slot="profile-create-fields"
          className="-mx-1 flex min-h-0 flex-col gap-3 overflow-y-auto px-1 pt-1 pb-2"
        >
          <motion.div
            data-slot="profile-name-field"
            className="flex items-center gap-2 rounded-xl bg-foreground/4 p-2 transition-[background-color,box-shadow] duration-150 focus-within:bg-foreground/6 focus-within:ring-2 focus-within:ring-foreground/10 hover:bg-foreground/5"
            {...rise(0)}
          >
            <label
              htmlFor="profile-name"
              className="flex min-w-0 flex-1 cursor-text items-center gap-2.5"
            >
              <ProfileGlyph
                seed={profileName}
                routed={!!selectedProxyId}
                locked={enablePassword}
                ephemeral={ephemeral}
                busy={isCreating}
              />
              <span className="sr-only">{t("createProfile.profileName")}</span>
              <Input
                id="profile-name"
                variant="bare"
                autoFocus
                autoComplete="off"
                spellCheck={false}
                value={profileName}
                onChange={(event) => setProfileName(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    void handleCreate();
                  }
                }}
                placeholder={t("createProfile.profileNamePlaceholder")}
                disabled={isCreating}
                className="h-9 text-base font-medium md:text-base"
              />
            </label>
            <span id={osLabelId} className="sr-only">
              {t("fingerprint.osLabel")}
            </span>
            <Popover open={osPopoverOpen} onOpenChange={setOsPopoverOpen}>
              <PopoverTrigger asChild>
                <button
                  id="profile-os"
                  type="button"
                  role="combobox"
                  aria-expanded={osPopoverOpen}
                  aria-controls={osListboxId}
                  aria-labelledby={`${osLabelId} ${osValueId}`}
                  className={cn(optionPill(crossOs), "shrink-0")}
                  disabled={isCreating}
                >
                  <PillIcon on={crossOs} animate={animateEntry}>
                    <SelectedOsIcon />
                  </PillIcon>
                  <PillLabel
                    id={osValueId}
                    label={getOSDisplayName(selectedOs)}
                    animate={animateEntry}
                  />
                  <LuChevronDown
                    className={cn(
                      "-mr-0.5 opacity-60 transition-transform duration-200 motion-reduce:transition-none",
                      osPopoverOpen && "rotate-180",
                    )}
                    aria-hidden="true"
                  />
                </button>
              </PopoverTrigger>
              <PopoverContent className="w-52 p-0" align="end" sideOffset={6}>
                <Command>
                  <CommandList id={osListboxId}>
                    <CommandGroup>
                      {OS_OPTIONS.map((os) => {
                        const locked = os !== hostOs && !crossOsUnlocked;
                        const Icon = getOSIcon(os);
                        return (
                          <CommandItem
                            key={os}
                            value={os}
                            disabled={locked}
                            onSelect={() => {
                              updateWayfernConfig("os", os);
                              setOsPopoverOpen(false);
                            }}
                          >
                            <LuCheck
                              className={cn(
                                "size-4 shrink-0",
                                selectedOs === os ? "opacity-100" : "opacity-0",
                              )}
                              aria-hidden="true"
                            />
                            <Icon
                              className="size-3.5 shrink-0"
                              aria-hidden="true"
                            />
                            <span className="truncate">
                              {getOSDisplayName(os)}
                            </span>
                            {locked && <ProBadge className="ml-auto" />}
                          </CommandItem>
                        );
                      })}
                    </CommandGroup>
                  </CommandList>
                </Command>
              </PopoverContent>
            </Popover>
          </motion.div>

          <motion.div
            className="flex flex-wrap items-center gap-1.5"
            {...rise(0.04)}
          >
            <span id={proxyLabelId} className="sr-only">
              {t("createProfile.proxy.title")}
            </span>
            <Popover open={proxyPopoverOpen} onOpenChange={setProxyPopoverOpen}>
              <PopoverTrigger asChild>
                <button
                  id="profile-proxy"
                  type="button"
                  role="combobox"
                  aria-expanded={proxyPopoverOpen}
                  aria-controls={proxyListboxId}
                  aria-labelledby={`${proxyLabelId} ${proxyValueId}`}
                  className={optionPill(!!selectedProxyId)}
                  disabled={isCreating}
                >
                  <PillIcon on={!!selectedProxyId} animate={animateEntry}>
                    {selectedVpn ? <LuShieldCheck /> : <LuGlobe />}
                  </PillIcon>
                  <PillLabel
                    id={proxyValueId}
                    label={proxyLabel}
                    animate={animateEntry}
                  />
                  <LuChevronDown
                    className={cn(
                      "-mr-0.5 opacity-60 transition-transform duration-200 motion-reduce:transition-none",
                      proxyPopoverOpen && "rotate-180",
                    )}
                    aria-hidden="true"
                  />
                </button>
              </PopoverTrigger>
              <PopoverContent className="w-72 p-0" align="start" sideOffset={6}>
                <Command>
                  <CommandInput placeholder={t("createProfile.proxy.search")} />
                  <CommandList id={proxyListboxId}>
                    <CommandEmpty>
                      {t("createProfile.proxy.notFound")}
                    </CommandEmpty>
                    <CommandGroup>
                      <CommandItem
                        value="__none__"
                        keywords={[
                          t("common.labels.none"),
                          t("createProfile.proxy.noProxy"),
                        ]}
                        onSelect={() => {
                          setSelectedProxyId(undefined);
                          setProxyPopoverOpen(false);
                        }}
                      >
                        <LuCheck
                          className={cn(
                            "size-4 shrink-0",
                            !selectedProxyId ? "opacity-100" : "opacity-0",
                          )}
                          aria-hidden="true"
                        />
                        {t("common.labels.none")}
                      </CommandItem>
                      {storedProxies.map((proxy) => (
                        <CommandItem
                          key={proxy.id}
                          value={proxy.id}
                          keywords={[proxy.name]}
                          onSelect={() => {
                            setSelectedProxyId(proxy.id);
                            setProxyPopoverOpen(false);
                          }}
                        >
                          <LuCheck
                            className={cn(
                              "size-4 shrink-0",
                              selectedProxyId === proxy.id
                                ? "opacity-100"
                                : "opacity-0",
                            )}
                            aria-hidden="true"
                          />
                          <span className="truncate">{proxy.name}</span>
                        </CommandItem>
                      ))}
                    </CommandGroup>
                    {vpnConfigs.length > 0 && (
                      <CommandGroup heading={t("proxies.management.tabVpns")}>
                        {vpnConfigs.map((vpn) => (
                          <CommandItem
                            key={vpn.id}
                            value={`vpn-${vpn.id}`}
                            keywords={[vpn.name, "VPN", "WireGuard"]}
                            onSelect={() => {
                              setSelectedProxyId(`vpn-${vpn.id}`);
                              setProxyPopoverOpen(false);
                            }}
                          >
                            <LuCheck
                              className={cn(
                                "size-4 shrink-0",
                                selectedProxyId === `vpn-${vpn.id}`
                                  ? "opacity-100"
                                  : "opacity-0",
                              )}
                              aria-hidden="true"
                            />
                            <Badge
                              variant="soft"
                              className="px-1 py-0 text-[10px] leading-tight"
                            >
                              WG
                            </Badge>
                            <span className="truncate">{vpn.name}</span>
                          </CommandItem>
                        ))}
                      </CommandGroup>
                    )}
                    <CommandSeparator
                      alwaysRender
                      className="my-1 bg-foreground/8"
                    />
                    <CommandGroup forceMount>
                      <CommandItem
                        value="__add_proxy__"
                        forceMount
                        onSelect={() => {
                          setProxyPopoverOpen(false);
                          setShowProxyForm(true);
                        }}
                      >
                        <GoPlus
                          className="size-4 shrink-0"
                          aria-hidden="true"
                        />
                        {t("createProfile.proxy.addProxy")}
                      </CommandItem>
                    </CommandGroup>
                  </CommandList>
                </Command>
              </PopoverContent>
            </Popover>

            <OptionToggle
              id="enable-password"
              checked={enablePassword}
              onCheckedChange={(checked) => {
                if (checked) {
                  setEnablePassword(true);
                  setEphemeral(false);
                } else clearPassword();
              }}
              disabled={isCreating}
              animate={animateEntry}
              icon={enablePassword ? <LuLock /> : <LuLockOpen />}
              label={t("profiles.passwordProtectedBadge")}
              hint={t("createProfile.passwordProtect.description")}
            />
            <OptionToggle
              id="ephemeral"
              checked={ephemeral}
              onCheckedChange={(checked) => {
                setEphemeral(checked);
                if (checked) clearPassword();
              }}
              disabled={isCreating}
              animate={animateEntry}
              icon={<LuFlame className={cn(ephemeral && "fill-current")} />}
              label={t("profiles.ephemeral")}
              hint={t("profiles.ephemeralDescription")}
            />
          </motion.div>

          {enablePassword && (
            <motion.div className="space-y-1.5" {...reveal}>
              <div className="grid gap-2 sm:grid-cols-2">
                <div>
                  <Label htmlFor="profile-password" className="sr-only">
                    {t("profilePassword.fields.newPassword")}
                  </Label>
                  <Input
                    id="profile-password"
                    variant="soft"
                    type="password"
                    value={password}
                    onChange={(event) => {
                      setPassword(event.target.value);
                      setPasswordError(null);
                    }}
                    placeholder={t("profilePassword.fields.newPassword")}
                    autoComplete="new-password"
                    aria-invalid={!!passwordError}
                    aria-describedby={
                      passwordError ? "profile-password-error" : undefined
                    }
                    disabled={isCreating}
                  />
                </div>
                <div>
                  <Label htmlFor="profile-password-confirm" className="sr-only">
                    {t("profilePassword.fields.confirm")}
                  </Label>
                  <Input
                    id="profile-password-confirm"
                    variant="soft"
                    type="password"
                    value={passwordConfirm}
                    onChange={(event) => {
                      setPasswordConfirm(event.target.value);
                      setPasswordError(null);
                    }}
                    placeholder={t("profilePassword.fields.confirm")}
                    autoComplete="new-password"
                    aria-invalid={!!passwordError}
                    aria-describedby={
                      passwordError ? "profile-password-error" : undefined
                    }
                    disabled={isCreating}
                  />
                </div>
              </div>
              {passwordError && (
                <motion.p
                  key={passwordError}
                  id="profile-password-error"
                  role="alert"
                  className="px-1 text-xs text-destructive-text"
                  initial={animateEntry ? { x: 0 } : false}
                  animate={animateEntry ? { x: [0, -4, 4, -2, 0] } : { x: 0 }}
                  transition={{ duration: 0.32, ease: MOTION_EASE_OUT }}
                >
                  {passwordError}
                </motion.p>
              )}
            </motion.div>
          )}

          {crossOs && (
            <motion.p
              role="status"
              className={cn(STATUS_CLASS, "items-start")}
              {...reveal}
            >
              <LuTriangleAlert
                className="mt-0.5 size-3.5 shrink-0"
                aria-hidden="true"
              />
              {t("fingerprint.crossOsWarning")}
            </motion.p>
          )}

          {downloading ? (
            <motion.p role="status" className={STATUS_CLASS} {...reveal}>
              <LuLoaderCircle
                className="size-3.5 shrink-0 animate-spin"
                aria-hidden="true"
              />
              {t("createProfile.version.downloading", {
                browser: "Wayfern",
                version: latestVersion,
              })}
            </motion.p>
          ) : (
            !creatableVersion &&
            (isLoadingReleaseTypes ? (
              <motion.p role="status" className={STATUS_CLASS} {...reveal}>
                <LuLoaderCircle
                  className="size-3.5 shrink-0 animate-spin"
                  aria-hidden="true"
                />
                {t("createProfile.version.fetching")}
              </motion.p>
            ) : releaseTypesError ? (
              <motion.div
                role="alert"
                className={cn(STATUS_CLASS, "bg-destructive/10 py-1.5 pr-1.5")}
                {...reveal}
              >
                <p className="flex-1 text-destructive-text">
                  {t("createProfile.version.fetchError")}
                </p>
                <Button
                  onClick={() => void loadReleaseTypes()}
                  size="sm"
                  variant="soft"
                  className="h-7 rounded-md text-xs"
                >
                  {t("common.buttons.retry")}
                </Button>
              </motion.div>
            ) : latestVersion ? (
              <motion.div
                className={cn(STATUS_CLASS, "py-1.5 pr-1.5")}
                {...reveal}
              >
                <p className="flex-1">
                  {t("createProfile.version.needsDownload", {
                    browser: "Wayfern",
                    version: latestVersion,
                  })}
                </p>
                <Button
                  onClick={() => void handleDownload()}
                  size="sm"
                  className="h-7 rounded-md text-xs"
                >
                  {t("common.buttons.download")}
                </Button>
              </motion.div>
            ) : (
              <p
                role="status"
                className={cn(STATUS_CLASS, "text-warning-text")}
              >
                {t("createProfile.platformUnavailable", {
                  browser: "Wayfern",
                })}
              </p>
            ))
          )}

          {creatingQuickly && (
            <motion.div
              role="status"
              className={cn(
                STATUS_CLASS,
                "items-start bg-warning/10 text-warning-text",
              )}
              {...reveal}
            >
              <LuTriangleAlert
                className="mt-0.5 size-3.5 shrink-0"
                aria-hidden="true"
              />
              <div className="space-y-0.5">
                <p className="font-medium">
                  {t("createProfile.hourlyLimit.warningTitle")}
                </p>
                <p>{t("createProfile.hourlyLimit.warning")}</p>
              </div>
            </motion.div>
          )}

          <div id={advancedOptionsId} hidden={!advancedOpen}>
            <AnimatedDisclosureContent
              open={advancedOpen}
              className="space-y-4 pt-2"
            >
              <SoftFields>
                <div className="space-y-4">
                  <div className="space-y-1.5">
                    <Label
                      htmlFor="launch-hook-url"
                      className={FIELD_LABEL_CLASS}
                    >
                      {t("createProfile.launchHook.label")}
                    </Label>
                    <Input
                      id="launch-hook-url"
                      variant="soft"
                      value={launchHook}
                      onChange={(event) => setLaunchHook(event.target.value)}
                      placeholder={t("createProfile.launchHook.placeholder")}
                      disabled={isCreating}
                    />
                  </div>
                  <div
                    className={cn(
                      "grid gap-4",
                      extensionGroups.length > 0 && "sm:grid-cols-2",
                    )}
                  >
                    <div className="min-w-0 space-y-1.5">
                      <Label
                        htmlFor="profile-dns-blocklist"
                        className={FIELD_LABEL_CLASS}
                      >
                        {t("dnsBlocklist.title")}
                      </Label>
                      <Select
                        value={dnsBlocklist || "none"}
                        onValueChange={(value) =>
                          setDnsBlocklist(value === "none" ? "" : value)
                        }
                        disabled={isCreating}
                      >
                        <SelectTrigger
                          id="profile-dns-blocklist"
                          variant="soft"
                          className="w-full"
                        >
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="none">
                            {t("dnsBlocklist.none")}
                          </SelectItem>
                          {DNS_BLOCKLIST_LEVELS.map((level) => (
                            <SelectItem key={level.value} value={level.value}>
                              {t(level.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    {extensionGroups.length > 0 && (
                      <div className="min-w-0 space-y-1.5">
                        <Label
                          htmlFor="profile-extension-group"
                          className={FIELD_LABEL_CLASS}
                        >
                          {t("extensions.extensionGroup")}
                        </Label>
                        <Select
                          value={selectedExtensionGroupId ?? "none"}
                          onValueChange={(value) =>
                            setSelectedExtensionGroupId(
                              value === "none" ? undefined : value,
                            )
                          }
                          disabled={isCreating}
                        >
                          <SelectTrigger
                            id="profile-extension-group"
                            variant="soft"
                            className="w-full"
                          >
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="none">
                              {t("profileInfo.values.none")}
                            </SelectItem>
                            {extensionGroups.map((group) => (
                              <SelectItem key={group.id} value={group.id}>
                                {group.name} ({group.extension_ids.length})
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                    )}
                  </div>
                  <div className="space-y-4 pt-1">
                    {creatableVersion && (
                      <p className="text-xs text-muted-foreground">
                        {t("createProfile.version.available", {
                          browser: "Wayfern",
                          version: creatableVersion,
                        })}
                      </p>
                    )}
                    {creatableVersion &&
                      latestVersion &&
                      !downloadedVersions.includes(latestVersion) &&
                      !downloading && (
                        <div className={cn(STATUS_CLASS, "py-1.5 pr-1.5")}>
                          <p className="flex-1">
                            {t("createProfile.version.upgradeAvailable", {
                              browser: "Wayfern",
                              version: latestVersion,
                            })}
                          </p>
                          <Button
                            onClick={() => void handleDownload()}
                            size="sm"
                            variant="soft"
                            className="h-7 rounded-md text-xs"
                            disabled={isCreating}
                          >
                            {t("common.buttons.download")}
                          </Button>
                        </div>
                      )}
                    <WayfernConfigForm
                      className="[&_[data-slot=select-trigger]]:w-full"
                      config={wayfernConfig}
                      onConfigChange={updateWayfernConfig}
                      isCreating
                      forceAdvanced
                      readOnly={isCreating}
                      crossOsUnlocked={crossOsUnlocked}
                      limitedMode={!crossOsUnlocked}
                      osPicker={false}
                      profileVersion={creatableVersion}
                      profileBrowser="wayfern"
                    />
                  </div>
                </div>
              </SoftFields>
            </AnimatedDisclosureContent>
          </div>
        </div>

        <DialogFooter className="items-center justify-between">
          <Button
            variant="subtle"
            size="sm"
            className="-ml-2 gap-1 rounded-lg px-2"
            aria-expanded={advancedOpen}
            aria-controls={advancedOptionsId}
            onClick={() => setAdvancedOpen((open) => !open)}
            disabled={isCreating}
          >
            <AnimatedDisclosureChevron open={advancedOpen}>
              <LuChevronRight className="size-4" />
            </AnimatedDisclosureChevron>
            {t("createProfile.advancedOptions")}
          </Button>
          <div className="ml-auto flex items-center gap-2">
            <Button
              variant="subtle"
              onClick={handleClose}
              disabled={isCreating}
            >
              {t("common.buttons.cancel")}
            </Button>
            {hourlyCapReached ? (
              <Tooltip
                onOpenChange={(open) => {
                  if (open) void loadAllowance();
                }}
              >
                <TooltipTrigger asChild>
                  <span data-slot="hourly-limit" className="inline-flex">
                    {createButton}
                  </span>
                </TooltipTrigger>
                <TooltipContent side="top" sideOffset={6} className="max-w-64">
                  {t("createProfile.hourlyLimit.reached", {
                    limit: allowance.per_hour,
                    minutes: Math.max(
                      1,
                      Math.ceil((allowance.retry_after_secs ?? 60) / 60),
                    ),
                  })}
                </TooltipContent>
              </Tooltip>
            ) : (
              createButton
            )}
          </div>
        </DialogFooter>
      </DialogContent>
      <ProxyFormDialog
        isOpen={showProxyForm}
        onClose={() => setShowProxyForm(false)}
      />
    </Dialog>
  );
}

const ENTRY_SPRING = {
  type: "spring" as const,
  stiffness: 420,
  damping: 34,
  mass: 0.7,
};

/** Mirror of `profile_generation_limiter::Allowance`. */
interface ProfileCreationAllowance {
  enforcement: "hard" | "soft";
  per_hour: number;
  used: number;
  retry_after_secs: number | null;
}

const STATUS_CLASS =
  "flex items-center gap-2 rounded-lg bg-foreground/4 px-3 py-2 text-xs text-muted-foreground";

const FIELD_LABEL_CLASS = "text-xs font-medium text-muted-foreground";

function optionPill(on: boolean) {
  return cn(
    "inline-flex h-8 max-w-full min-w-0 cursor-pointer items-center gap-1.5 rounded-full px-3 text-sm font-medium outline-none select-none transition-[background-color,color,box-shadow,transform] duration-150 ease-[var(--ease-out)] focus-visible:ring-2 focus-visible:ring-ring/50 active:scale-[0.97] disabled:pointer-events-none disabled:opacity-50 motion-reduce:active:scale-100 [&_svg]:size-3.5 [&_svg]:shrink-0",
    on
      ? "bg-primary/10 text-foreground [&_svg]:text-primary-text"
      : "bg-foreground/5 text-muted-foreground hover:bg-foreground/8 hover:text-foreground",
  );
}

/**
 * False on the first render, true after it. A pill's parts spring when their
 * option changes, never as the dialog opens: the first frame shows every
 * option at rest, so a stalled frame cannot leave one half drawn.
 */
function useMounted() {
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
  }, []);
  return mounted.current;
}

/** A pill's icon springs in whenever the option it marks changes state. */
function PillIcon({
  on,
  animate,
  children,
}: {
  on: boolean;
  animate: boolean;
  children: React.ReactNode;
}) {
  const mounted = useMounted();
  return (
    <motion.span
      key={String(on)}
      aria-hidden="true"
      className="inline-flex"
      initial={
        animate && mounted ? { scale: 0.4, rotate: on ? -24 : 24 } : false
      }
      animate={{ scale: 1, rotate: 0 }}
      transition={{ type: "spring", stiffness: 560, damping: 22 }}
    >
      {children}
    </motion.span>
  );
}

/** The chosen route's name, which rises into place when it changes. */
function PillLabel({
  id,
  label,
  animate,
}: {
  id: string;
  label: string;
  animate: boolean;
}) {
  const mounted = useMounted();
  return (
    <motion.span
      key={label}
      id={id}
      className="min-w-0 truncate"
      initial={animate && mounted ? { y: 4 } : false}
      animate={{ y: 0 }}
      transition={ENTRY_SPRING}
    >
      {label}
    </motion.span>
  );
}

/**
 * An on/off option drawn as a pill. It is a real checkbox underneath, so it
 * keeps the checkbox role, state and keyboard behavior; the hint explains the
 * option without adding text to the form.
 */
function OptionToggle({
  id,
  checked,
  onCheckedChange,
  disabled,
  animate,
  icon,
  label,
  hint,
}: {
  id: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  disabled: boolean;
  animate: boolean;
  icon: React.ReactNode;
  label: string;
  hint: string;
}) {
  return (
    <Tooltip delayDuration={600}>
      <TooltipTrigger asChild>
        <CheckboxPrimitive.Root
          id={id}
          checked={checked}
          onCheckedChange={(state) => onCheckedChange(state === true)}
          disabled={disabled}
          className={optionPill(checked)}
        >
          <PillIcon on={checked} animate={animate}>
            {icon}
          </PillIcon>
          <span className="truncate">{label}</span>
        </CheckboxPrimitive.Root>
      </TooltipTrigger>
      <TooltipContent side="top" sideOffset={6} className="max-w-72">
        {hint}
      </TooltipContent>
    </Tooltip>
  );
}
