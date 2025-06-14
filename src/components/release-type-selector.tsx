"use client";

import { LoadingButton } from "@/components/loading-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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
import { useState } from "react";
import { LuDownload } from "react-icons/lu";
import { LuCheck, LuChevronsUpDown } from "react-icons/lu";

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
      <Popover open={popoverOpen} onOpenChange={setPopoverOpen} modal={true}>
        <PopoverTrigger asChild>
          <Button
            variant="outline"
            role="combobox"
            aria-expanded={popoverOpen}
            className="justify-between w-full"
          >
            {selectedDisplayText}
            <LuChevronsUpDown className="ml-2 w-4 h-4 opacity-50 shrink-0" />
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-[300px] p-0">
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
