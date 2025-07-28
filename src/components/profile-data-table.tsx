"use client";

import {
  type ColumnDef,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  type SortingState,
  useReactTable,
} from "@tanstack/react-table";
import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { CiCircleCheck } from "react-icons/ci";
import { IoEllipsisHorizontal } from "react-icons/io5";
import { LuChevronDown, LuChevronUp } from "react-icons/lu";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ScrollArea } from "@/components/ui/scroll-area";
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
import { useBrowserState } from "@/hooks/use-browser-support";
import { useTableSorting } from "@/hooks/use-table-sorting";
import {
  getBrowserDisplayName,
  getBrowserIcon,
  getCurrentOS,
} from "@/lib/browser-utils";
import { trimName } from "@/lib/name-utils";
import { cn } from "@/lib/utils";
import type { BrowserProfile, StoredProxy } from "@/types";
import { Input } from "./ui/input";
import { Label } from "./ui/label";

interface ProfilesDataTableProps {
  data: BrowserProfile[];
  onLaunchProfile: (profile: BrowserProfile) => void | Promise<void>;
  onKillProfile: (profile: BrowserProfile) => void | Promise<void>;
  onProxySettings: (profile: BrowserProfile) => void;
  onDeleteProfile: (profile: BrowserProfile) => void | Promise<void>;
  onRenameProfile: (oldName: string, newName: string) => Promise<void>;
  onChangeVersion: (profile: BrowserProfile) => void;
  onConfigureCamoufox?: (profile: BrowserProfile) => void;
  runningProfiles: Set<string>;
  isUpdating?: (browser: string) => boolean;
  onReloadProxyData?: () => void | Promise<void>;
  onDeleteSelectedProfiles?: (profileNames: string[]) => Promise<void>;
  onAssignProfilesToGroup?: (profileNames: string[]) => void;
  selectedGroupId?: string | null;
  selectedProfiles?: string[];
  onSelectedProfilesChange?: (profiles: string[]) => void;
}

export function ProfilesDataTable({
  data,
  onLaunchProfile,
  onKillProfile,
  onProxySettings,
  onDeleteProfile,
  onRenameProfile,
  onChangeVersion,
  onConfigureCamoufox,
  runningProfiles,
  isUpdating = () => false,
  onDeleteSelectedProfiles: _onDeleteSelectedProfiles,
  onAssignProfilesToGroup,
  selectedGroupId,
  selectedProfiles: externalSelectedProfiles = [],
  onSelectedProfilesChange,
}: ProfilesDataTableProps) {
  const { getTableSorting, updateSorting, isLoaded } = useTableSorting();
  const [sorting, setSorting] = React.useState<SortingState>([]);
  const [profileToRename, setProfileToRename] =
    React.useState<BrowserProfile | null>(null);
  const [newProfileName, setNewProfileName] = React.useState("");
  const [renameError, setRenameError] = React.useState<string | null>(null);
  const [profileToDelete, setProfileToDelete] =
    React.useState<BrowserProfile | null>(null);
  const [isDeleting, setIsDeleting] = React.useState(false);

  const [storedProxies, setStoredProxies] = React.useState<StoredProxy[]>([]);
  const [selectedProfiles, setSelectedProfiles] = React.useState<Set<string>>(
    new Set(externalSelectedProfiles),
  );
  const [showCheckboxes, setShowCheckboxes] = React.useState(false);

  // Helper function to check if a profile has a proxy
  const hasProxy = React.useCallback(
    (profile: BrowserProfile): boolean => {
      if (!profile.proxy_id) return false;
      const proxy = storedProxies.find((p) => p.id === profile.proxy_id);
      return proxy !== undefined;
    },
    [storedProxies],
  );

  // Helper function to get proxy info for a profile
  const getProxyInfo = React.useCallback(
    (profile: BrowserProfile): StoredProxy | null => {
      if (!profile.proxy_id) return null;
      return storedProxies.find((p) => p.id === profile.proxy_id) ?? null;
    },
    [storedProxies],
  );

  // Helper function to get proxy name for display
  const getProxyDisplayName = React.useCallback(
    (profile: BrowserProfile): string => {
      if (!profile.proxy_id) return "Disabled";
      const proxy = storedProxies.find((p) => p.id === profile.proxy_id);
      return proxy?.name ?? "Unknown Proxy";
    },
    [storedProxies],
  );

  // Filter data by selected group
  const filteredData = React.useMemo(() => {
    if (!selectedGroupId) return data;
    if (selectedGroupId === "default") {
      return data.filter((profile) => !profile.group_id);
    }
    return data.filter((profile) => profile.group_id === selectedGroupId);
  }, [data, selectedGroupId]);

  // Use shared browser state hook
  const browserState = useBrowserState(
    filteredData,
    runningProfiles,
    isUpdating,
  );

  // Load stored proxies
  const loadStoredProxies = React.useCallback(async () => {
    try {
      const proxiesList = await invoke<StoredProxy[]>("get_stored_proxies");
      setStoredProxies(proxiesList);
    } catch (error) {
      console.error("Failed to load stored proxies:", error);
    }
  }, []);

  React.useEffect(() => {
    if (browserState.isClient) {
      void loadStoredProxies();
    }
  }, [browserState.isClient, loadStoredProxies]);

  // Sync external selected profiles with internal state
  React.useEffect(() => {
    const newSet = new Set(externalSelectedProfiles);
    setSelectedProfiles(newSet);
    setShowCheckboxes(newSet.size > 0);
  }, [externalSelectedProfiles]);

  // Update local sorting state when settings are loaded
  React.useEffect(() => {
    if (isLoaded && browserState.isClient) {
      setSorting(getTableSorting());
    }
  }, [isLoaded, getTableSorting, browserState.isClient]);

  // Handle sorting changes
  const handleSortingChange = React.useCallback(
    (updater: React.SetStateAction<SortingState>) => {
      if (!browserState.isClient) return;
      const newSorting =
        typeof updater === "function" ? updater(sorting) : updater;
      setSorting(newSorting);
      updateSorting(newSorting);
    },
    [browserState.isClient, sorting, updateSorting],
  );

  const handleRename = async () => {
    if (!profileToRename || !newProfileName.trim()) return;

    try {
      await onRenameProfile(profileToRename.name, newProfileName.trim());
      setProfileToRename(null);
      setNewProfileName("");
      setRenameError(null);
    } catch (error) {
      setRenameError(
        error instanceof Error ? error.message : "Failed to rename profile",
      );
    }
  };

  const handleDelete = async () => {
    if (!profileToDelete) return;

    setIsDeleting(true);
    try {
      await onDeleteProfile(profileToDelete);
      setProfileToDelete(null);
    } catch (error) {
      console.error("Failed to delete profile:", error);
    } finally {
      setIsDeleting(false);
    }
  };

  // Handle icon/checkbox click
  const handleIconClick = React.useCallback(
    (profileName: string) => {
      setShowCheckboxes(true);
      setSelectedProfiles((prev) => {
        const newSet = new Set(prev);
        if (newSet.has(profileName)) {
          newSet.delete(profileName);
        } else {
          newSet.add(profileName);
        }

        // Hide checkboxes if no profiles are selected
        if (newSet.size === 0) {
          setShowCheckboxes(false);
        }

        // Notify parent component
        if (onSelectedProfilesChange) {
          onSelectedProfilesChange(Array.from(newSet));
        }

        return newSet;
      });
    },
    [onSelectedProfilesChange],
  );

  // Handle checkbox change
  const handleCheckboxChange = React.useCallback(
    (profileName: string, checked: boolean) => {
      setSelectedProfiles((prev) => {
        const newSet = new Set(prev);
        if (checked) {
          newSet.add(profileName);
        } else {
          newSet.delete(profileName);
        }

        // Hide checkboxes if no profiles are selected
        if (newSet.size === 0) {
          setShowCheckboxes(false);
        }

        // Notify parent component
        if (onSelectedProfilesChange) {
          onSelectedProfilesChange(Array.from(newSet));
        }

        return newSet;
      });
    },
    [onSelectedProfilesChange],
  );

  // Handle select all checkbox
  const handleToggleAll = React.useCallback(
    (checked: boolean) => {
      const newSet = checked
        ? new Set(filteredData.map((profile) => profile.name))
        : new Set<string>();

      setSelectedProfiles(newSet);
      setShowCheckboxes(checked);

      // Notify parent component
      if (onSelectedProfilesChange) {
        onSelectedProfilesChange(Array.from(newSet));
      }
    },
    [filteredData, onSelectedProfilesChange],
  );

  const columns: ColumnDef<BrowserProfile>[] = React.useMemo(
    () => [
      {
        id: "select",
        header: () => (
          <span>
            <Checkbox
              checked={
                selectedProfiles.size === filteredData.length &&
                filteredData.length !== 0
              }
              onCheckedChange={(value) => handleToggleAll(!!value)}
              aria-label="Select all"
              className="cursor-pointer"
            />
          </span>
        ),
        cell: ({ row }) => {
          const profile = row.original;
          const browser = profile.browser;
          const IconComponent = getBrowserIcon(browser);
          const isSelected = selectedProfiles.has(profile.name);

          if (showCheckboxes || isSelected) {
            return (
              <span className="w-4 h-4 flex items-center justify-center">
                <Checkbox
                  checked={isSelected}
                  onCheckedChange={(value) =>
                    handleCheckboxChange(profile.name, !!value)
                  }
                  aria-label="Select row"
                  className="w-4 h-4"
                />
              </span>
            );
          }

          return (
            <span className="relative flex items-center justify-center w-4 h-4">
              <button
                type="button"
                className="flex items-center justify-center cursor-pointer border-none p-0"
                onClick={() => handleIconClick(profile.name)}
                aria-label="Select profile"
              >
                <span className="w-4 h-4 group">
                  {IconComponent && (
                    <IconComponent className="w-4 h-4 group-hover:hidden" />
                  )}
                  <span className="peer border-input dark:bg-input/30 dark:data-[state=checked]:bg-primary size-4 shrink-0 rounded-[4px] border shadow-xs transition-shadow outline-none w-4 h-4 hidden group-hover:block pointer-events-none items-center justify-center duration-200" />
                </span>
              </button>
            </span>
          );
        },
        enableSorting: false,
        enableHiding: false,
        size: 40,
      },
      {
        id: "actions",
        cell: ({ row }) => {
          const profile = row.original;
          const isRunning =
            browserState.isClient && runningProfiles.has(profile.name);
          const canLaunch = browserState.canLaunchProfile(profile);
          const tooltipContent = browserState.getLaunchTooltipContent(profile);

          return (
            <div className="flex gap-2 items-center">
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="inline-flex">
                    <Button
                      variant={isRunning ? "destructive" : "default"}
                      size="sm"
                      disabled={!canLaunch}
                      className={cn(
                        "cursor-pointer",
                        !canLaunch && "opacity-50",
                      )}
                      onClick={() =>
                        void (isRunning
                          ? onKillProfile(profile)
                          : onLaunchProfile(profile))
                      }
                    >
                      {isRunning ? "Stop" : "Launch"}
                    </Button>
                  </span>
                </TooltipTrigger>
                {tooltipContent && (
                  <TooltipContent>{tooltipContent}</TooltipContent>
                )}
              </Tooltip>
            </div>
          );
        },
      },
      {
        accessorKey: "name",
        header: ({ column }) => {
          return (
            <Button
              variant="ghost"
              onClick={() =>
                column.toggleSorting(column.getIsSorted() === "asc")
              }
              className="h-auto p-0 font-semibold text-left justify-start"
            >
              Name
              {column.getIsSorted() === "asc" ? (
                <LuChevronUp className="ml-2 h-4 w-4" />
              ) : column.getIsSorted() === "desc" ? (
                <LuChevronDown className="ml-2 h-4 w-4" />
              ) : null}
            </Button>
          );
        },
        enableSorting: true,
        sortingFn: "alphanumeric",
        cell: ({ row }) => {
          const rawName: string = row.getValue("name");
          const name = getBrowserDisplayName(rawName);

          if (name.length < 20) {
            return <div className="font-medium text-left">{name}</div>;
          }

          return (
            <Tooltip>
              <TooltipTrigger asChild>
                <span>{trimName(name, 20)}</span>
              </TooltipTrigger>
              <TooltipContent>{name}</TooltipContent>
            </Tooltip>
          );
        },
      },
      {
        accessorKey: "browser",
        header: ({ column }) => {
          return (
            <Button
              variant="ghost"
              onClick={() =>
                column.toggleSorting(column.getIsSorted() === "asc")
              }
              className="h-auto p-0 font-semibold text-left justify-start"
            >
              Browser
              {column.getIsSorted() === "asc" ? (
                <LuChevronUp className="ml-2 h-4 w-4" />
              ) : column.getIsSorted() === "desc" ? (
                <LuChevronDown className="ml-2 h-4 w-4" />
              ) : null}
            </Button>
          );
        },
        cell: ({ row }) => {
          const browser: string = row.getValue("browser");
          const name = getBrowserDisplayName(browser);
          if (name.length < 20) {
            return (
              <div className="flex items-center">
                <span>{name}</span>
              </div>
            );
          }

          return (
            <Tooltip>
              <TooltipTrigger asChild>
                <span>{trimName(name, 20)}</span>
              </TooltipTrigger>
              <TooltipContent>{name}</TooltipContent>
            </Tooltip>
          );
        },
        enableSorting: true,
        sortingFn: (rowA, rowB, columnId) => {
          const browserA: string = rowA.getValue(columnId);
          const browserB: string = rowB.getValue(columnId);
          return getBrowserDisplayName(browserA).localeCompare(
            getBrowserDisplayName(browserB),
          );
        },
      },
      {
        accessorKey: "release_type",
        header: "Release",
        cell: ({ row }) => {
          const releaseType: string = row.getValue("release_type");
          const isNightly = releaseType === "nightly";
          return (
            <div className="flex items-center">
              <span
                className={`inline-flex items-center px-2 py-1 rounded-full text-xs font-medium ${
                  isNightly
                    ? "text-yellow-800 bg-yellow-100 dark:bg-yellow-900 dark:text-yellow-200"
                    : "text-green-800 bg-green-100 dark:bg-green-900 dark:text-green-200"
                }`}
              >
                {isNightly ? "Nightly" : "Stable"}
              </span>
            </div>
          );
        },
        enableSorting: true,
        sortingFn: (rowA, rowB, columnId) => {
          const releaseA: string = rowA.getValue(columnId);
          const releaseB: string = rowB.getValue(columnId);
          // Sort with "stable" before "nightly"
          if (releaseA === "stable" && releaseB === "nightly") return -1;
          if (releaseA === "nightly" && releaseB === "stable") return 1;
          return 0;
        },
      },
      {
        id: "proxy",
        header: "Proxy",
        cell: ({ row }) => {
          const profile = row.original;
          const profileHasProxy = hasProxy(profile);
          const proxyDisplayName = getProxyDisplayName(profile);
          const proxyInfo = getProxyInfo(profile);

          const tooltipText =
            profile.browser === "tor-browser"
              ? "Proxies are not supported for TOR browser"
              : profileHasProxy && proxyInfo
                ? `${proxyDisplayName}, ${proxyInfo.proxy_settings.proxy_type.toUpperCase()} (${
                    proxyInfo.proxy_settings.host
                  }:${proxyInfo.proxy_settings.port})`
                : "No proxy configured";

          return (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="flex gap-2 items-center">
                  {profileHasProxy && (
                    <CiCircleCheck className="w-4 h-4 text-green-500" />
                  )}
                  {proxyDisplayName.length > 10 ? (
                    <span className="text-sm truncate text-muted-foreground">
                      {proxyDisplayName.slice(0, 10)}...
                    </span>
                  ) : (
                    <span className="text-sm text-muted-foreground">
                      {profile.browser === "tor-browser"
                        ? "Not supported"
                        : proxyDisplayName}
                    </span>
                  )}
                </span>
              </TooltipTrigger>
              <TooltipContent>{tooltipText}</TooltipContent>
            </Tooltip>
          );
        },
      },
      {
        id: "settings",
        cell: ({ row }) => {
          const profile = row.original;
          const isRunning =
            browserState.isClient && runningProfiles.has(profile.name);
          const isBrowserUpdating =
            browserState.isClient && isUpdating(profile.browser);

          return (
            <div className="flex justify-end items-center">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    variant="ghost"
                    className="p-0 w-8 h-8"
                    disabled={!browserState.isClient}
                  >
                    <span className="sr-only">Open menu</span>
                    <IoEllipsisHorizontal className="w-4 h-4" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuLabel>Actions</DropdownMenuLabel>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem
                    onClick={() => {
                      onProxySettings(profile);
                    }}
                    disabled={!browserState.isClient || isBrowserUpdating}
                  >
                    Configure Proxy
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    onClick={() => {
                      if (onAssignProfilesToGroup) {
                        onAssignProfilesToGroup([profile.name]);
                      }
                    }}
                    disabled={!browserState.isClient || isBrowserUpdating}
                  >
                    Assign to Group
                  </DropdownMenuItem>
                  {profile.browser === "camoufox" && onConfigureCamoufox && (
                    <DropdownMenuItem
                      onClick={() => {
                        onConfigureCamoufox(profile);
                      }}
                      disabled={
                        !browserState.isClient || isRunning || isBrowserUpdating
                      }
                    >
                      Configure Camoufox
                    </DropdownMenuItem>
                  )}
                  {!["chromium", "zen", "camoufox"].includes(
                    profile.browser,
                  ) && (
                    <DropdownMenuItem
                      onClick={() => {
                        onChangeVersion(profile);
                      }}
                      disabled={
                        !browserState.isClient || isRunning || isBrowserUpdating
                      }
                    >
                      Switch Release
                    </DropdownMenuItem>
                  )}
                  <DropdownMenuItem
                    onClick={() => {
                      setProfileToRename(profile);
                      setNewProfileName(profile.name);
                    }}
                    disabled={
                      !browserState.isClient || isRunning || isBrowserUpdating
                    }
                  >
                    Rename
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    onClick={() => {
                      setProfileToDelete(profile);
                    }}
                    disabled={
                      !browserState.isClient || isRunning || isBrowserUpdating
                    }
                  >
                    Delete
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          );
        },
      },
    ],
    [
      showCheckboxes,
      selectedProfiles,
      handleToggleAll,
      handleCheckboxChange,
      handleIconClick,
      runningProfiles,
      browserState,
      hasProxy,
      getProxyDisplayName,
      getProxyInfo,
      onProxySettings,
      onLaunchProfile,
      onKillProfile,
      onConfigureCamoufox,
      onChangeVersion,
      onAssignProfilesToGroup,
      isUpdating,
      filteredData.length,
    ],
  );

  const table = useReactTable({
    data: filteredData,
    columns,
    state: {
      sorting,
    },
    onSortingChange: handleSortingChange,
    getSortedRowModel: getSortedRowModel(),
    getCoreRowModel: getCoreRowModel(),
  });

  const platform = getCurrentOS();

  return (
    <>
      <ScrollArea
        className={cn(
          "rounded-md border",
          platform === "macos" ? "h-[340px]" : "h-[280px]",
        )}
      >
        <Table>
          <TableHeader>
            {table.getHeaderGroups().map((headerGroup) => (
              <TableRow key={headerGroup.id}>
                {headerGroup.headers.map((header) => {
                  return (
                    <TableHead key={header.id}>
                      {header.isPlaceholder
                        ? null
                        : flexRender(
                            header.column.columnDef.header,
                            header.getContext(),
                          )}
                    </TableHead>
                  );
                })}
              </TableRow>
            ))}
          </TableHeader>
          <TableBody>
            {table.getRowModel().rows?.length ? (
              table.getRowModel().rows.map((row) => (
                <TableRow
                  key={row.id}
                  data-state={row.getIsSelected() && "selected"}
                  className="hover:bg-accent/50"
                >
                  {row.getVisibleCells().map((cell) => (
                    <TableCell key={cell.id}>
                      {flexRender(
                        cell.column.columnDef.cell,
                        cell.getContext(),
                      )}
                    </TableCell>
                  ))}
                </TableRow>
              ))
            ) : (
              <TableRow>
                <TableCell
                  colSpan={columns.length}
                  className="h-24 text-center"
                >
                  No profiles found.
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </ScrollArea>

      <Dialog
        open={profileToRename !== null}
        onOpenChange={(open) => {
          if (!open) {
            setProfileToRename(null);
            setNewProfileName("");
            setRenameError(null);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Rename Profile</DialogTitle>
          </DialogHeader>
          <div className="grid gap-4 py-4">
            <div className="grid grid-cols-4 gap-4 items-center">
              <Label htmlFor="name" className="text-right">
                Name
              </Label>
              <Input
                id="name"
                value={newProfileName}
                onChange={(e) => {
                  setNewProfileName(e.target.value);
                }}
                className="col-span-3"
              />
            </div>
            {renameError && (
              <p className="text-sm text-red-600">{renameError}</p>
            )}
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => {
                setProfileToRename(null);
              }}
            >
              Cancel
            </Button>
            <Button onClick={() => void handleRename()}>Save</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <DeleteConfirmationDialog
        isOpen={profileToDelete !== null}
        onClose={() => setProfileToDelete(null)}
        onConfirm={handleDelete}
        title="Delete Profile"
        description={`This action cannot be undone. This will permanently delete the profile "${profileToDelete?.name}" and all its associated data.`}
        confirmButtonText="Delete Profile"
        isLoading={isDeleting}
      />
    </>
  );
}
