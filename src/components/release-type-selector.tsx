"use client";

import { useState } from "react";
import { LuCheck, LuChevronsUpDown, LuDownload } from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import { Badge } from "@/components/ui/badge";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import type { BrowserReleaseTypes } from "@/types";
import { RippleButton } from "./ui/ripple";

interface ReleaseTypeSelectorProps {
  selectedReleaseType: "stable" | "nightly" | null;
  onReleaseTypeSelect: (releaseType: "stable" | "nightly" | null) => void;
  availableReleaseTypes: BrowserReleaseTypes;
  browser: string;
  isDownloading: boolean;
  onDownload: () => void;
  placeholder?: string;
  showDownloadButton?: boolean;
  downloadedVersions?: string[];
}

export function ReleaseTypeSelector({
  selectedReleaseType,
  onReleaseTypeSelect,
  availableReleaseTypes,
  browser,
  isDownloading,
  onDownload,
  placeholder = "Select release type...",
  showDownloadButton = true,
  downloadedVersions = [],
}: ReleaseTypeSelectorProps) {
  const [popoverOpen, setPopoverOpen] = useState(false);

  const releaseOptions = [
    ...(availableReleaseTypes.stable
      ? [{ type: "stable" as const, version: availableReleaseTypes.stable }]
      : []),
    ...(availableReleaseTypes.nightly && browser !== "chromium"
      ? [{ type: "nightly" as const, version: availableReleaseTypes.nightly }]
      : []),
  ];

  // Only show dropdown if there are multiple release types available
  const showDropdown = releaseOptions.length > 1;

  // If only one release type is available, auto-select it
  if (!showDropdown && releaseOptions.length === 1 && !selectedReleaseType) {
    setTimeout(() => {
      onReleaseTypeSelect(releaseOptions[0].type);
    }, 0);
  }

  const selectedDisplayText = selectedReleaseType
    ? selectedReleaseType === "stable"
      ? "Stable"
      : "Nightly"
    : placeholder;

  const selectedVersion =
    selectedReleaseType === "stable"
      ? availableReleaseTypes.stable
      : selectedReleaseType === "nightly"
        ? availableReleaseTypes.nightly
        : null;

  const isVersionDownloaded =
    selectedVersion && downloadedVersions.includes(selectedVersion);

  return (
    <div className="space-y-4">
      {showDropdown ? (
        <Popover open={popoverOpen} onOpenChange={setPopoverOpen} modal={true}>
          <PopoverTrigger asChild>
            <RippleButton
              variant="outline"
              role="combobox"
              aria-expanded={popoverOpen}
              className="justify-between w-full"
            >
              {selectedDisplayText}
              <LuChevronsUpDown className="ml-2 w-4 h-4 opacity-50 shrink-0" />
            </RippleButton>
          </PopoverTrigger>
          <PopoverContent className="p-0">
            <Command>
              <CommandEmpty>No release types available.</CommandEmpty>
              <CommandList>
                <CommandGroup>
                  {releaseOptions.map((option) => {
                    const isDownloaded = downloadedVersions.includes(
                      option.version,
                    );
                    return (
                      <CommandItem
                        key={option.type}
                        value={option.type}
                        onSelect={(currentValue) => {
                          const selectedType = currentValue as
                            | "stable"
                            | "nightly";
                          onReleaseTypeSelect(
                            selectedType === selectedReleaseType
                              ? null
                              : selectedType,
                          );
                          setPopoverOpen(false);
                        }}
                      >
                        <LuCheck
                          className={cn(
                            "mr-2 h-4 w-4",
                            selectedReleaseType === option.type
                              ? "opacity-100"
                              : "opacity-0",
                          )}
                        />
                        <div className="flex gap-2 items-center">
                          <span className="capitalize">{option.type}</span>
                          {option.type === "nightly" && (
                            <Badge variant="secondary" className="text-xs">
                              Nightly
                            </Badge>
                          )}
                          <Badge variant="outline" className="text-xs">
                            {option.version}
                          </Badge>
                          {isDownloaded && (
                            <Badge variant="default" className="text-xs">
                              Downloaded
                            </Badge>
                          )}
                        </div>
                      </CommandItem>
                    );
                  })}
                </CommandGroup>
              </CommandList>
            </Command>
          </PopoverContent>
        </Popover>
      ) : (
        // Show a simple display when only one release type is available
        releaseOptions.length === 1 && (
          <div className="flex gap-2 justify-center items-center p-3 rounded-md border bg-muted/50">
            <span className="text-sm font-medium capitalize">
              {releaseOptions[0].type}
            </span>
            {releaseOptions[0].type === "nightly" && (
              <Badge variant="secondary" className="text-xs">
                Nightly
              </Badge>
            )}
            <Badge variant="outline" className="text-xs">
              {releaseOptions[0].version}
            </Badge>
            {downloadedVersions.includes(releaseOptions[0].version) && (
              <Badge variant="default" className="text-xs">
                Downloaded
              </Badge>
            )}
          </div>
        )
      )}

      {showDownloadButton &&
        selectedReleaseType &&
        selectedVersion &&
        !isVersionDownloaded && (
          <LoadingButton
            isLoading={isDownloading}
            onClick={() => {
              onDownload();
            }}
            variant="outline"
            className="w-full"
          >
            <LuDownload className="mr-2 w-4 h-4" />
            {isDownloading ? "Downloading..." : "Download Browser"}
          </LoadingButton>
        )}
    </div>
  );
}
