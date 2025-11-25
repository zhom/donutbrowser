import { getFlagIconClass } from "@/lib/flag-utils";
import { cn } from "@/lib/utils";

interface FlagIconProps {
  countryCode?: string;
  className?: string;
  squared?: boolean;
}

export function FlagIcon({
  countryCode,
  className,
  squared = false,
}: FlagIconProps) {
  if (!countryCode) {
    return null;
  }

  const flagClass = getFlagIconClass(countryCode);
  if (!flagClass) {
    return null;
  }

  return <span className={cn(flagClass, squared && "fis", className)} />;
}
