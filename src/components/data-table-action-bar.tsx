"use client";

import type { RowData } from "@tanstack/react-table";
import type { LegacyTable as Table } from "@tanstack/react-table/legacy";
import { motion, useReducedMotion } from "motion/react";
import * as React from "react";
import * as ReactDOM from "react-dom";
import { useTranslation } from "react-i18next";
import { LuX } from "react-icons/lu";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useInputModality } from "@/hooks/use-input-modality";
import { MOTION_EASE_OUT } from "@/lib/motion";
import { cn } from "@/lib/utils";

interface DataTableActionBarProps<TData extends RowData>
  extends React.ComponentProps<typeof motion.div> {
  table: Table<TData>;
  visible?: boolean;
  portalContainer?: Element | DocumentFragment | null;
}

function DataTableActionBar<TData extends RowData>({
  table,
  visible: visibleProp,
  portalContainer: portalContainerProp,
  children,
  className,
  ...props
}: DataTableActionBarProps<TData>) {
  const reduceMotion = useReducedMotion();
  const inputModality = useInputModality();
  const [mounted, setMounted] = React.useState(false);
  const barRef = React.useRef<HTMLDivElement>(null);
  const visible =
    visibleProp ?? table.getFilteredSelectedRowModel().rows.length > 0;
  React.useLayoutEffect(() => {
    setMounted(true);
  }, []);
  React.useLayoutEffect(() => {
    const bar = barRef.current;
    if (!mounted || !visible || !bar) return;
    const style = document.documentElement.style;
    const measure = () =>
      style.setProperty("--table-action-bar-height", `${bar.offsetHeight}px`);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(bar);
    return () => {
      observer.disconnect();
      style.removeProperty("--table-action-bar-height");
    };
  }, [mounted, visible]);

  React.useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (
        event.key === "Escape" &&
        !event.defaultPrevented &&
        !document.querySelector('[data-slot="dialog-content"]')
      ) {
        table.toggleAllRowsSelected(false);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [table]);

  const portalContainer =
    portalContainerProp ?? (mounted ? globalThis.document.body : null);

  if (!portalContainer) return null;

  if (!visible) return null;

  return ReactDOM.createPortal(
    <motion.div
      ref={barRef}
      data-slot="table-action-bar"
      role="toolbar"
      aria-orientation="horizontal"
      initial={reduceMotion || inputModality === "keyboard" ? false : { y: 6 }}
      animate={{ y: 0 }}
      transition={{
        duration: reduceMotion || inputModality === "keyboard" ? 0 : 0.16,
        ease: MOTION_EASE_OUT,
      }}
      className={cn(
        "surface-material-popover fixed inset-x-0 bottom-6 z-50 mx-auto flex w-fit max-w-[calc(100%-2rem)] flex-wrap items-center justify-center gap-1.5 rounded-xl p-1.5 text-foreground shadow-lg ring-1 ring-foreground/8",
        className,
      )}
      {...props}
    >
      {children}
    </motion.div>,
    portalContainer,
  );
}

interface DataTableActionBarActionProps
  extends React.ComponentProps<typeof Button> {
  tooltip?: string;
  label?: string;
  isPending?: boolean;
}

function DataTableActionBarAction({
  size = "sm",
  tooltip,
  label,
  isPending,
  disabled,
  className,
  children,
  ...props
}: DataTableActionBarActionProps) {
  const trigger = (
    <Button
      variant="soft"
      size={size}
      className={cn(
        "gap-1.5 rounded-lg text-xs [&>svg]:size-3.5",
        size === "icon" ? "size-7" : "h-7",
        className,
      )}
      disabled={disabled || isPending}
      aria-label={label ?? tooltip}
      {...props}
    >
      {isPending ? (
        <div className="size-3.5 animate-spin rounded-full border border-current border-t-transparent" />
      ) : (
        children
      )}
      {size !== "icon" && (label ?? tooltip) && <span>{label ?? tooltip}</span>}
    </Button>
  );

  if (!tooltip) return trigger;

  return (
    <Tooltip>
      <TooltipTrigger asChild>{trigger}</TooltipTrigger>
      <TooltipContent
        sideOffset={6}
        className="border bg-accent font-semibold text-accent-foreground [&>span]:hidden"
      >
        <p>{tooltip}</p>
      </TooltipContent>
    </Tooltip>
  );
}

interface DataTableActionBarSelectionProps<TData extends RowData> {
  table: Table<TData>;
}

function DataTableActionBarSelection<TData extends RowData>({
  table,
}: DataTableActionBarSelectionProps<TData>) {
  const { t } = useTranslation();
  const onClearSelection = React.useCallback(() => {
    table.toggleAllRowsSelected(false);
  }, [table]);

  return (
    <div className="flex h-7 items-center pr-1 pl-2">
      <span className="text-xs font-medium whitespace-nowrap tabular-nums">
        {t("dataTableActionBar.selected", {
          count: table.getFilteredSelectedRowModel().rows.length,
        })}
      </span>
      <div className="mr-1 ml-2 h-4 w-px bg-foreground/10" />
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="subtle"
            size="icon"
            className="size-5 rounded-md"
            aria-label={t("dataTableActionBar.clearSelection")}
            onClick={onClearSelection}
          >
            <LuX className="size-3.5" />
          </Button>
        </TooltipTrigger>
        <TooltipContent
          sideOffset={10}
          className="flex items-center gap-2 border bg-accent px-2 py-1 font-semibold text-accent-foreground [&>span]:hidden"
        >
          <p>{t("dataTableActionBar.clearSelection")}</p>
          <kbd className="rounded border bg-background px-1.5 py-px font-mono text-[0.7rem] font-normal text-foreground shadow-xs select-none">
            <abbr title={t("common.keys.escape")} className="no-underline">
              Esc
            </abbr>
          </kbd>
        </TooltipContent>
      </Tooltip>
    </div>
  );
}

export {
  DataTableActionBar,
  DataTableActionBarAction,
  DataTableActionBarSelection,
};
