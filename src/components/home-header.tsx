import { FaDownload } from "react-icons/fa";
import { FiWifi } from "react-icons/fi";
import { GoGear, GoKebabHorizontal, GoPlus } from "react-icons/go";
import { LuTrash2, LuUsers } from "react-icons/lu";
import { Logo } from "./icons/logo";
import { Button } from "./ui/button";
import { CardTitle } from "./ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";

type Props = {
  selectedProfiles: string[];
  onBulkGroupAssignment: () => void;
  onBulkDelete: () => Promise<void>;
  onSettingsDialogOpen: (open: boolean) => void;
  onProxyManagementDialogOpen: (open: boolean) => void;
  onGroupManagementDialogOpen: (open: boolean) => void;
  onImportProfileDialogOpen: (open: boolean) => void;
  onCreateProfileDialogOpen: (open: boolean) => void;
};

const HomeHeader = ({
  selectedProfiles,
  onBulkGroupAssignment,
  onBulkDelete,
  onSettingsDialogOpen,
  onProxyManagementDialogOpen,
  onGroupManagementDialogOpen,
  onImportProfileDialogOpen,
  onCreateProfileDialogOpen,
}: Props) => {
  return (
    <div className="flex justify-between items-center">
      <div className="flex items-center gap-3">
        <Logo className="w-10 h-10" />
        {selectedProfiles.length > 0 ? (
          <div className="flex items-center gap-3">
            <span className="text-sm font-medium">
              {selectedProfiles.length} profile
              {selectedProfiles.length !== 1 ? "s" : ""} selected
            </span>
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={onBulkGroupAssignment}
                className="flex gap-2 items-center"
              >
                <LuUsers className="w-4 h-4" />
                Assign to Group
              </Button>
              <Button
                variant="destructive"
                size="sm"
                onClick={() => void onBulkDelete()}
                className="flex gap-2 items-center"
              >
                <LuTrash2 className="w-4 h-4" />
                Delete Selected
              </Button>
            </div>
          </div>
        ) : (
          <CardTitle>Donut</CardTitle>
        )}
      </div>
      <div className="flex gap-2 items-center">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              size="sm"
              variant="outline"
              className="flex gap-2 items-center"
            >
              <GoKebabHorizontal className="w-4 h-4" />
            </Button>
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
                className="flex gap-2 items-center"
              >
                <GoPlus className="w-4 h-4" />
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent>Create a new profile</TooltipContent>
        </Tooltip>
      </div>
    </div>
  );
};

export default HomeHeader;
