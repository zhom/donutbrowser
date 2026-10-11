import { useConfigList } from "@/hooks/use-config-list";
import type { ProfileTemplate } from "@/types";

/** Every profile template, in order, kept current by `profile-templates-changed`. */
export function useProfileTemplates(): ProfileTemplate[] {
  return useConfigList<ProfileTemplate>(
    "list_profile_templates",
    "profile-templates-changed",
  );
}
