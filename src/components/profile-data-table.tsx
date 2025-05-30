"use client";

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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
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
import { getBrowserDisplayName, getBrowserIcon } from "@/lib/browser-utils";
import type { BrowserProfile } from "@/types";
import {
  type ColumnDef,
  type SortingState,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
} from "@tanstack/react-table";
import * as React from "react";
import { CiCircleCheck } from "react-icons/ci";
import { IoEllipsisHorizontal } from "react-icons/io5";
import { LuChevronDown, LuChevronUp } from "react-icons/lu";
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
  runningProfiles: Set<string>;
  isUpdating?: (browser: string) => boolean;
}

export function ProfilesDataTable({
  data,
  onLaunchProfile,
  onKillProfile,
  onProxySettings,
  onDeleteProfile,
  onRenameProfile,
  onChangeVersion,
  runningProfiles,
  isUpdating = () => false,
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

  // Ensure we're on the client side to prevent hydration mismatches
  React.useEffect(() => {
    setIsClient(true);
  }, []);

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
            <div className="flex items-center gap-2">
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
              className="h-auto p-0 font-semibold hover:bg-transparent"
            >
              Profile
              {isSorted === "asc" && <LuChevronUp className="ml-2 h-4 w-4" />}
              {isSorted === "desc" && (
                <LuChevronDown className="ml-2 h-4 w-4" />
              )}
              {!isSorted && (
                <LuChevronDown className="ml-2 h-4 w-4 opacity-50" />
              )}
            </Button>
          );
        },
        enableSorting: true,
        sortingFn: "alphanumeric",
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
              className="h-auto p-0 font-semibold hover:bg-transparent"
            >
              Browser
              {isSorted === "asc" && <LuChevronUp className="ml-2 h-4 w-4" />}
              {isSorted === "desc" && (
                <LuChevronDown className="ml-2 h-4 w-4" />
              )}
              {!isSorted && (
                <LuChevronDown className="ml-2 h-4 w-4 opacity-50" />
              )}
            </Button>
          );
        },
        cell: ({ row }) => {
          const browser: string = row.getValue("browser");
          const IconComponent = getBrowserIcon(browser);
          return (
            <div className="flex items-center gap-2">
              {IconComponent && <IconComponent className="h-4 w-4" />}
              <span>{getBrowserDisplayName(browser)}</span>
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
        accessorKey: "version",
        header: "Version",
      },
      {
        id: "status",
        header: ({ column }) => {
          const isSorted = column.getIsSorted();
          return (
            <Button
              variant="ghost"
              onClick={() => {
                column.toggleSorting(column.getIsSorted() === "asc");
              }}
              className="h-auto p-0 font-semibold hover:bg-transparent"
            >
              Status
              {isSorted === "asc" && <LuChevronUp className="ml-2 h-4 w-4" />}
              {isSorted === "desc" && (
                <LuChevronDown className="ml-2 h-4 w-4" />
              )}
              {!isSorted && (
                <LuChevronDown className="ml-2 h-4 w-4 opacity-50" />
              )}
            </Button>
          );
        },
        cell: ({ row }) => {
          const profile = row.original;
          const isRunning = isClient && runningProfiles.has(profile.name);
          return (
            <div className="flex flex-col gap-1">
              <Badge
                variant={isRunning ? "default" : "secondary"}
                className="text-xs w-fit"
              >
                {isClient ? (isRunning ? "Running" : "Stopped") : "Loading..."}
              </Badge>
              {isClient && isRunning && profile.process_id && (
                <span className="text-xs text-muted-foreground">
                  PID: {profile.process_id}
                </span>
              )}
            </div>
          );
        },
        enableSorting: true,
        sortingFn: (rowA, rowB) => {
          // If not on client, sort by name only to ensure consistency
          if (!isClient) {
            return rowA.original.name.localeCompare(rowB.original.name);
          }

          const isRunningA = runningProfiles.has(rowA.original.name);
          const isRunningB = runningProfiles.has(rowB.original.name);

          // Running profiles come first, then stopped ones
          // Secondary sort by profile name
          if (isRunningA === isRunningB) {
            return rowA.original.name.localeCompare(rowB.original.name);
          }
          return isRunningA ? -1 : 1;
        },
      },
      {
        id: "proxy",
        header: "Proxy",
        cell: ({ row }) => {
          const profile = row.original;
          const hasProxy = profile.proxy?.enabled;
          return (
            <Tooltip>
              <TooltipTrigger>
                <div className="flex items-center gap-2">
                  {hasProxy && (
                    <CiCircleCheck className="h-4 w-4 text-green-500" />
                  )}
                  <span className="text-sm text-muted-foreground">
                    {hasProxy ? profile.proxy?.proxy_type : "Disabled"}
                  </span>
                </div>
              </TooltipTrigger>
              <TooltipContent>
                {hasProxy
                  ? `${profile.proxy?.proxy_type.toUpperCase()} proxy enabled (${
                      profile.proxy?.host
                    }:${profile.proxy?.port})`
                  : "No proxy configured"}
              </TooltipContent>
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
            <div className="flex items-center justify-end">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    variant="ghost"
                    className="h-8 w-8 p-0"
                    disabled={!isClient}
                  >
                    <span className="sr-only">Open menu</span>
                    <IoEllipsisHorizontal className="h-4 w-4" />
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
                    Configure proxy
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    onClick={() => {
                      onChangeVersion(profile);
                    }}
                    disabled={!isClient || isRunning || isBrowserUpdating}
                  >
                    Change version
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    onClick={() => {
                      setProfileToRename(profile);
                      setNewProfileName(profile.name);
                    }}
                    disabled={!isClient || isRunning || isBrowserUpdating}
                  >
                    Rename profile
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    onClick={() => {
                      setProfileToDelete(profile);
                      setDeleteConfirmationName("");
                    }}
                    className="text-red-600"
                    disabled={!isClient || isRunning || isBrowserUpdating}
                  >
                    Delete profile
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

  return (
    <>
      <div className="rounded-md border">
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
      </div>

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
            <div className="grid grid-cols-4 items-center gap-4">
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
