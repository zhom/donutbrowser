import { FaDownload } from "react-icons/fa";
import { FiWifi } from "react-icons/fi";
import { GoGear, GoKebabHorizontal, GoPlus } from "react-icons/go";
import { LuCloud, LuSearch, LuUsers, LuX } from "react-icons/lu";
import { Logo } from "./icons/logo";
import { Button } from "./ui/button";
import { CardTitle } from "./ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu";
import { Input } from "./ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";

type Props = {
  onSettingsDialogOpen: (open: boolean) => void;
  onProxyManagementDialogOpen: (open: boolean) => void;
  onGroupManagementDialogOpen: (open: boolean) => void;
  onImportProfileDialogOpen: (open: boolean) => void;
  onCreateProfileDialogOpen: (open: boolean) => void;
  onSyncConfigDialogOpen: (open: boolean) => void;
  searchQuery: string;
  onSearchQueryChange: (query: string) => void;
};

const HomeHeader = ({
  onSettingsDialogOpen,
  onProxyManagementDialogOpen,
  onGroupManagementDialogOpen,
  onImportProfileDialogOpen,
  onCreateProfileDialogOpen,
  onSyncConfigDialogOpen,
  searchQuery,
  onSearchQueryChange,
}: Props) => {
  const handleLogoClick = () => {
    // Trigger the same URL handling logic as if the URL came from the system
    const event = new CustomEvent("url-open-request", {
      detail: "https://donutbrowser.com",
    });
    window.dispatchEvent(event);
  };
  return (
    <div className="flex justify-between items-center mt-6">
      <div className="flex gap-3 items-center">
        <button
          type="button"
          className="p-1 cursor-pointer"
          title="Open donutbrowser.com"
          onClick={handleLogoClick}
        >
          <Logo className="w-10 h-10 transition-transform duration-300 ease-out will-change-transform hover:scale-110" />
        </button>
        <CardTitle>Donut</CardTitle>
      </div>
      <div className="flex gap-2 items-center">
        <div className="relative">
          <Input
            type="text"
            placeholder="Search profiles..."
            value={searchQuery}
            onChange={(e) => onSearchQueryChange(e.target.value)}
            className="pr-8 pl-10 w-48"
          />
          <LuSearch className="absolute left-3 top-1/2 w-4 h-4 transform -translate-y-1/2 text-muted-foreground" />
          {searchQuery && (
            <button
              type="button"
              onClick={() => onSearchQueryChange("")}
              className="absolute right-2 top-1/2 p-1 rounded-sm transition-colors transform -translate-y-1/2 hover:bg-accent"
              aria-label="Clear search"
            >
              <LuX className="w-4 h-4 text-muted-foreground hover:text-foreground" />
            </button>
          )}
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <span>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span>
                    <Button
                      size="sm"
                      variant="outline"
                      className="flex gap-2 items-center h-[36px]"
                    >
                      <GoKebabHorizontal className="w-4 h-4" />
                    </Button>
                  </span>
                </TooltipTrigger>
                <TooltipContent>More actions</TooltipContent>
              </Tooltip>
            </span>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              onClick={() => {
                onSettingsDialogOpen(true);
              }}
            >
              <GoGear className="mr-2 w-4 h-4" />
              Settings
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => {
                onProxyManagementDialogOpen(true);
              }}
            >
              <FiWifi className="mr-2 w-4 h-4" />
              Proxies
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => {
                onGroupManagementDialogOpen(true);
              }}
            >
              <LuUsers className="mr-2 w-4 h-4" />
              Groups
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => {
                onSyncConfigDialogOpen(true);
              }}
            >
              <LuCloud className="mr-2 w-4 h-4" />
              Sync Service
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => {
                onImportProfileDialogOpen(true);
              }}
            >
              <FaDownload className="mr-2 w-4 h-4" />
              Import Profile
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <Tooltip>
          <TooltipTrigger asChild>
            <span>
              <Button
                size="sm"
                onClick={() => {
                  onCreateProfileDialogOpen(true);
                }}
                className="flex gap-2 items-center h-[36px]"
              >
                <GoPlus className="w-4 h-4" />
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent
            arrowOffset={-8}
            style={{ transform: "translateX(-8px)" }}
          >
            Create a new profile
          </TooltipContent>
        </Tooltip>
      </div>
    </div>
  );
};

export default HomeHeader;
