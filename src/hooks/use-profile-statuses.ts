import { useConfigList } from "@/hooks/use-config-list";
import type { ProfileStatus } from "@/types";

/** Every profile status, in order, kept current by `profile-statuses-changed`. */
export function useProfileStatuses(): ProfileStatus[] {
  return useConfigList<ProfileStatus>(
    "list_profile_statuses",
    "profile-statuses-changed",
  );
}
