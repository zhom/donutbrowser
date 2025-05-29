"use client";

import { LoadingButton } from "@/components/loading-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import { useState } from "react";
import { LuDownload } from "react-icons/lu";
import { LuCheck, LuChevronsUpDown } from "react-icons/lu";
import { ScrollArea } from "./ui/scroll-area";

interface GithubRelease {
  tag_name: string;
  assets: Array<{
    name: string;
    browser_download_url: string;
    hash?: string;
  }>;
  published_at: string;
  is_alpha: boolean;
}

interface VersionSelectorProps {
  selectedVersion: string | null;
  onVersionSelect: (version: string | null) => void;
  availableVersions: GithubRelease[];
  downloadedVersions: string[];
  isDownloading: boolean;
  onDownload: () => void;
  placeholder?: string;
  showDownloadButton?: boolean;
}

export function VersionSelector({
  selectedVersion,
  onVersionSelect,
  availableVersions,
  downloadedVersions,
  isDownloading,
  onDownload,
  placeholder = "Select version...",
  showDownloadButton = true,
}: VersionSelectorProps) {
  const [versionPopoverOpen, setVersionPopoverOpen] = useState(false);

  const isVersionDownloaded = selectedVersion
    ? downloadedVersions.includes(selectedVersion)
    : false;

  return (
    <div className="space-y-4">
      <Popover
        open={versionPopoverOpen}
        onOpenChange={setVersionPopoverOpen}
        modal={true}
      >
        <PopoverTrigger asChild>
          <Button
            variant="outline"
            role="combobox"
            aria-expanded={versionPopoverOpen}
            className="justify-between w-full"
          >
            {selectedVersion ?? placeholder}
            <LuChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-[300px] p-0">
          <Command>
            <CommandInput placeholder="Search versions..." />
            <CommandEmpty>No versions found.</CommandEmpty>
            <CommandList>
              <ScrollArea
                className={
                  "[&>[data-radix-scroll-area-viewport]]:max-h-[200px]"
                }
              >
                <CommandGroup>
                  {availableVersions.map((version) => {
                    const isDownloaded = downloadedVersions.includes(
                      version.tag_name,
                    );
                    return (
                      <CommandItem
                        key={version.tag_name}
                        value={version.tag_name}
                        onSelect={(currentValue) => {
                          onVersionSelect(
                            currentValue === selectedVersion
                              ? null
                              : currentValue,
                          );
                          setVersionPopoverOpen(false);
                        }}
                      >
                        <LuCheck
                          className={cn(
                            "mr-2 h-4 w-4",
                            selectedVersion === version.tag_name
                              ? "opacity-100"
                              : "opacity-0",
                          )}
                        />
                        <div className="flex items-center gap-2">
                          <span>{version.tag_name}</span>
                          {version.is_alpha && (
                            <Badge variant="secondary" className="text-xs">
                              Alpha
                            </Badge>
                          )}
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
              </ScrollArea>
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>

      {/* Download Button */}
      {showDownloadButton && selectedVersion && !isVersionDownloaded && (
        <LoadingButton
          isLoading={isDownloading}
          onClick={() => {
            onDownload();
          }}
          variant="outline"
          className="w-full"
        >
          <LuDownload className="mr-2 h-4 w-4" />
          {isDownloading ? "Downloading..." : "Download Browser"}
        </LoadingButton>
      )}
    </div>
  );
}
