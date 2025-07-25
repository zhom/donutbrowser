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
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
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
import { useTableSorting } from "@/hooks/use-table-sorting";
import {
  getBrowserDisplayName,
  getBrowserIcon,
  getCurrentOS,
} from "@/lib/browser-utils";
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
  onReloadProxyData,
}: ProfilesDataTableProps) {
  const { getTableSorting, updateSorting, isLoaded } = useTableSorting();
  const [sorting, setSorting] = React.useState<SortingState>([]);
  const [profileToRename, setProfileToRename] =
    React.useState<BrowserProfile | null>(null);
  const [newProfileName, setNewProfileName] = React.useState("");
  const [renameError, setRenameError] = React.useState<string | null>(null);
  const [profileToDelete, setProfileToDelete] =
    React.useState<BrowserProfile | null>(null);
  const [deleteConfirmationName, setDeleteConfirmationName] =
    React.useState("");
  const [deleteError, setDeleteError] = React.useState<string | null>(null);
  const [isClient, setIsClient] = React.useState(false);
  const [storedProxies, setStoredProxies] = React.useState<StoredProxy[]>([]);

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

  // Ensure we're on the client side to prevent hydration mismatches
  React.useEffect(() => {
    setIsClient(true);
  }, []);

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
    if (isClient) {
      void loadStoredProxies();
    }
  }, [isClient, loadStoredProxies]);

  // Reload proxy data when requested from parent
  React.useEffect(() => {
    if (onReloadProxyData) {
      void loadStoredProxies();
    }
  }, [onReloadProxyData, loadStoredProxies]);

  // Update local sorting state when settings are loaded
  React.useEffect(() => {
    if (isLoaded && isClient) {
      setSorting(getTableSorting());
    }
  }, [isLoaded, getTableSorting, isClient]);

  // Handle sorting changes
  const handleSortingChange = React.useCallback(
    (updater: React.SetStateAction<SortingState>) => {
      if (!isClient) return;
      const newSorting =
        typeof updater === "function" ? updater(sorting) : updater;
      setSorting(newSorting);
      updateSorting(newSorting);
    },
    [sorting, updateSorting, isClient],
  );

  const handleRename = async () => {
    if (!profileToRename || !newProfileName.trim()) return;

    try {
      await onRenameProfile(profileToRename.name, newProfileName.trim());
      setProfileToRename(null);
      setNewProfileName("");
      setRenameError(null);
    } catch (err) {
      setRenameError(err as string);
    }
  };

  const handleDelete = async () => {
    if (!profileToDelete || !deleteConfirmationName.trim()) return;

    if (deleteConfirmationName.trim() !== profileToDelete.name) {
      setDeleteError(
        "Profile name doesn't match. Please type the exact name to confirm deletion.",
      );
      return;
    }

    try {
      await onDeleteProfile(profileToDelete);
      setProfileToDelete(null);
      setDeleteConfirmationName("");
      setDeleteError(null);
    } catch (err) {
      setDeleteError(err as string);
    }
  };

  const columns: ColumnDef<BrowserProfile>[] = React.useMemo(
    () => [
      {
        id: "actions",
        cell: ({ row }) => {
          const profile = row.original;
          const isRunning = isClient && runningProfiles.has(profile.name);
          const isBrowserUpdating = isClient && isUpdating(profile.browser);

          // Check if any TOR browser profile is running
          const isTorBrowser = profile.browser === "tor-browser";
          const anyTorRunning =
            isClient &&
            data.some(
              (p) => p.browser === "tor-browser" && runningProfiles.has(p.name),
            );
          const shouldDisableTorStart =
            isTorBrowser && !isRunning && anyTorRunning;

          const isDisabled = shouldDisableTorStart || isBrowserUpdating;

          return (
            <div className="flex gap-2 items-center">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant={isRunning ? "destructive" : "default"}
                    size="sm"
                    disabled={!isClient || isDisabled}
                    onClick={() =>
                      void (isRunning
                        ? onKillProfile(profile)
                        : onLaunchProfile(profile))
                    }
                  >
                    {isRunning ? "Stop" : "Launch"}
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  {!isClient
                    ? "Loading..."
                    : isRunning
                      ? "Click to forcefully stop the browser"
                      : isBrowserUpdating
                        ? `${profile.browser} is being updated. Please wait for the update to complete.`
                        : shouldDisableTorStart
                          ? "Only one TOR browser instance can run at a time. Stop the running TOR browser first."
                          : "Click to launch the browser"}
                </TooltipContent>
              </Tooltip>
            </div>
          );
        },
      },
      {
        accessorKey: "name",
        header: ({ column }) => {
          const isSorted = column.getIsSorted();
          return (
            <Button
              variant="ghost"
              onClick={() => {
                column.toggleSorting(column.getIsSorted() === "asc");
              }}
              className="p-0 h-auto font-semibold hover:bg-transparent"
            >
              Profile
              {isSorted === "asc" && <LuChevronUp className="ml-2 w-4 h-4" />}
              {isSorted === "desc" && (
                <LuChevronDown className="ml-2 w-4 h-4" />
              )}
              {!isSorted && (
                <LuChevronDown className="ml-2 w-4 h-4 opacity-50" />
              )}
            </Button>
          );
        },
        enableSorting: true,
        sortingFn: "alphanumeric",
        cell: ({ row }) => {
          const profile = row.original;
          return profile.name.length > 15 ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="truncate">{profile.name.slice(0, 15)}...</span>
              </TooltipTrigger>
              <TooltipContent>{profile.name}</TooltipContent>
            </Tooltip>
          ) : (
            profile.name
          );
        },
      },
      {
        accessorKey: "browser",
        header: ({ column }) => {
          const isSorted = column.getIsSorted();
          return (
            <Button
              variant="ghost"
              onClick={() => {
                column.toggleSorting(column.getIsSorted() === "asc");
              }}
              className="p-0 h-auto font-semibold hover:bg-transparent"
            >
              Browser
              {isSorted === "asc" && <LuChevronUp className="ml-2 w-4 h-4" />}
              {isSorted === "desc" && (
                <LuChevronDown className="ml-2 w-4 h-4" />
              )}
              {!isSorted && (
                <LuChevronDown className="ml-2 w-4 h-4 opacity-50" />
              )}
            </Button>
          );
        },
        cell: ({ row }) => {
          const browser: string = row.getValue("browser");
          const IconComponent = getBrowserIcon(browser);
          const browserDisplayName = getBrowserDisplayName(browser);
          return browserDisplayName.length > 15 ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <div className="flex gap-2 items-center">
                  {IconComponent && <IconComponent className="w-4 h-4" />}
                  <span>{browserDisplayName.slice(0, 15)}...</span>
                </div>
              </TooltipTrigger>
              <TooltipContent>{browserDisplayName}</TooltipContent>
            </Tooltip>
          ) : (
            <div className="flex gap-2 items-center">
              {IconComponent && <IconComponent className="w-4 h-4" />}
              <span>{browserDisplayName}</span>
            </div>
          );
        },
        enableSorting: true,
        sortingFn: (rowA, rowB, columnId) => {
          const browserA = getBrowserDisplayName(rowA.getValue(columnId));
          const browserB = getBrowserDisplayName(rowB.getValue(columnId));
          return browserA.localeCompare(browserB);
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
              <TooltipTrigger>
                <div className="flex gap-2 items-center">
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
                </div>
              </TooltipTrigger>
              <TooltipContent>{tooltipText}</TooltipContent>
            </Tooltip>
          );
        },
      },
      // Update the settings column to use the confirmation dialog
      {
        id: "settings",
        cell: ({ row }) => {
          const profile = row.original;
          const isRunning = isClient && runningProfiles.has(profile.name);
          const isBrowserUpdating = isClient && isUpdating(profile.browser);
          return (
            <div className="flex justify-end items-center">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    variant="ghost"
                    className="p-0 w-8 h-8"
                    disabled={!isClient}
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
                    disabled={!isClient || isBrowserUpdating}
                  >
                    Configure Proxy
                  </DropdownMenuItem>
                  {profile.browser === "camoufox" && onConfigureCamoufox && (
                    <DropdownMenuItem
                      onClick={() => {
                        onConfigureCamoufox(profile);
                      }}
                      disabled={!isClient || isRunning || isBrowserUpdating}
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
                      disabled={!isClient || isRunning || isBrowserUpdating}
                    >
                      Switch Release
                    </DropdownMenuItem>
                  )}
                  <DropdownMenuItem
                    onClick={() => {
                      setProfileToRename(profile);
                      setNewProfileName(profile.name);
                    }}
                    disabled={!isClient || isRunning || isBrowserUpdating}
                  >
                    Rename
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    onClick={() => {
                      setProfileToDelete(profile);
                      setDeleteConfirmationName("");
                    }}
                    className="text-red-600"
                    disabled={!isClient || isRunning || isBrowserUpdating}
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
      isClient,
      runningProfiles,
      isUpdating,
      data,
      onLaunchProfile,
      onKillProfile,
      onProxySettings,
      onChangeVersion,
      onConfigureCamoufox,
      getProxyInfo,
      hasProxy,
      getProxyDisplayName,
    ],
  );

  const table = useReactTable({
    data,
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
          platform === "macos" ? "h-[380px]" : "h-[320px]",
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
            {table.getRowModel().rows.length ? (
              table.getRowModel().rows.map((row) => (
                <TableRow
                  key={row.id}
                  data-state={row.getIsSelected() && "selected"}
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

      <Dialog
        open={profileToDelete !== null}
        onOpenChange={(open) => {
          if (!open) {
            setProfileToDelete(null);
            setDeleteConfirmationName("");
            setDeleteError(null);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Profile</DialogTitle>
            <DialogDescription>
              This action cannot be undone. This will permanently delete the
              profile &quot;{profileToDelete?.name}&quot; and all its associated
              data.
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-4 py-4">
            <div className="grid gap-2">
              <Label htmlFor="delete-confirmation">
                Please type <strong>{profileToDelete?.name}</strong> to confirm:
              </Label>
              <Input
                id="delete-confirmation"
                value={deleteConfirmationName}
                onChange={(e) => {
                  setDeleteConfirmationName(e.target.value);
                  setDeleteError(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    void handleDelete();
                  }
                }}
                placeholder="Type the profile name here"
              />
            </div>
            {deleteError && (
              <p className="text-sm text-red-600">{deleteError}</p>
            )}
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => {
                setProfileToDelete(null);
                setDeleteConfirmationName("");
                setDeleteError(null);
              }}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              disabled={
                !deleteConfirmationName.trim() ||
                deleteConfirmationName !== profileToDelete?.name
              }
            >
              Delete Profile
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
