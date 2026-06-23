import { cn } from "@/lib/utils";

export function ProBadge({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        "rounded bg-primary px-1 py-0.5 text-[10px] font-semibold text-primary-foreground",
        className,
      )}
    >
      PRO
    </span>
  );
}
