import { cn } from "@/lib/utils";

export function ProBadge({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        "text-[10px] font-semibold px-1 py-0.5 rounded bg-primary text-primary-foreground",
        className,
      )}
    >
      PRO
    </span>
  );
}
