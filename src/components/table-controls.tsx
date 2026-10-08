"use client";

import type { RowData } from "@tanstack/react-table";
import type {
  LegacyColumn as Column,
  LegacyHeader as Header,
  LegacyRow as Row,
  LegacyTable as Table,
} from "@tanstack/react-table/legacy";
import { useState } from "react";
import { flushSync } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  LuArrowDown,
  LuArrowUp,
  LuArrowUpDown,
  LuCheck,
  LuChevronsUp,
  LuListFilter,
  LuPlus,
  LuSearch,
  LuSettings2,
  LuX,
} from "react-icons/lu";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { TableView } from "@/hooks/use-table-preferences";
import { selectTableRange, type TableFilterValues } from "@/lib/table-tools";
import { cn } from "@/lib/utils";

/**
 * Class names shared by the profile table and the management tables. The
 * shapes and tints live in `.data-table` (globals.css); these strip the base
 * table primitives' borders and row fills so they cannot paint under them.
 */
export const DATA_TABLE_CLASSES = {
  table: "data-table table-fixed",
  header: "sticky top-0 z-10 bg-background [&_tr]:border-0",
  headerRow: "border-0 hover:bg-transparent",
  head: "relative text-xs font-medium text-muted-foreground",
  row: "border-0 hover:bg-transparent data-[state=selected]:bg-transparent",
  cell: "py-0 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
} as const;

export interface TableColumnLabel {
  id: string;
  label: string;
  hideable?: boolean;
}
export interface FilterFacet {
  id: string;
  label: string;
  options: { value: string; label: string; count: number }[];
}

export function TableFilterControls({
  query,
  onQueryChange,
  values,
  onValuesChange,
  facets,
  count,
  total,
  onReset,
  showSearch = true,
}: {
  query: string;
  onQueryChange: (value: string) => void;
  values: TableFilterValues;
  onValuesChange: (values: TableFilterValues) => void;
  facets: FilterFacet[];
  count: number;
  total: number;
  onReset: () => void;
  showSearch?: boolean;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const active =
    Object.values(values).some((items) => items.length > 0) || query.length > 0;
  return (
    <div
      data-slot="table-filters"
      className="flex min-w-0 flex-wrap items-center gap-1.5"
    >
      {showSearch && (
        <div className="relative">
          <LuSearch className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            variant="soft"
            aria-label={t("tables.search")}
            placeholder={t("tables.search")}
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            className="h-8 w-48 pl-8 text-xs md:text-xs"
          />
        </div>
      )}
      {facets.length > 0 && (
        <Popover open={open} onOpenChange={setOpen}>
          <PopoverTrigger asChild>
            <Button
              variant="soft"
              size="sm"
              className="h-8 gap-1.5 rounded-lg px-2.5 text-xs"
            >
              <LuListFilter className="size-3.5" />
              {t("tables.filters")}
            </Button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-72 p-0">
            <Command>
              <CommandInput placeholder={t("tables.findFilter")} />
              <CommandList>
                <CommandEmpty>{t("tables.noOptions")}</CommandEmpty>
                {facets.map((facet) => (
                  <CommandGroup key={facet.id} heading={facet.label}>
                    {facet.options.map((option) => {
                      const selected =
                        values[facet.id]?.includes(option.value) ?? false;
                      return (
                        <CommandItem
                          key={option.value}
                          value={`${facet.label} ${option.label}`}
                          aria-checked={selected}
                          onSelect={() =>
                            onValuesChange({
                              ...values,
                              [facet.id]: selected
                                ? values[facet.id].filter(
                                    (value) => value !== option.value,
                                  )
                                : [...(values[facet.id] ?? []), option.value],
                            })
                          }
                        >
                          <span
                            className={cn(
                              "grid size-4 place-items-center rounded-[4px] border border-foreground/25 transition-colors duration-100",
                              selected &&
                                "border-primary bg-primary text-primary-foreground",
                            )}
                          >
                            {selected && <LuCheck className="size-3" />}
                          </span>
                          <span className="flex-1 truncate">
                            {option.label}
                          </span>
                          <span className="text-xs text-muted-foreground tabular-nums">
                            {option.count}
                          </span>
                        </CommandItem>
                      );
                    })}
                  </CommandGroup>
                ))}
              </CommandList>
            </Command>
          </PopoverContent>
        </Popover>
      )}
      {facets.flatMap((facet) =>
        (values[facet.id] ?? []).map((value) => {
          const label =
            facet.options.find((option) => option.value === value)?.label ??
            value;
          return (
            <Button
              key={`${facet.id}:${value}`}
              variant="soft"
              size="sm"
              className="h-7 max-w-64 gap-1 rounded-full bg-primary/10 pr-2 pl-2.5 text-xs hover:bg-primary/12"
              aria-label={t("tables.removeFilter", {
                filter: `${facet.label}: ${label}`,
              })}
              onClick={() =>
                onValuesChange({
                  ...values,
                  [facet.id]: values[facet.id].filter((item) => item !== value),
                })
              }
            >
              <span className="truncate">
                <span className="text-muted-foreground">{facet.label}:</span>{" "}
                {label}
              </span>
              <LuX className="size-3 shrink-0 opacity-60" />
            </Button>
          );
        }),
      )}
      {active && (
        <Button
          variant="subtle"
          size="sm"
          className="h-7 rounded-lg px-2 text-xs"
          onClick={onReset}
        >
          {t("tables.clearFilters")}
        </Button>
      )}
      <span
        data-slot="table-result-count"
        role="status"
        className="px-1 text-xs whitespace-nowrap text-muted-foreground tabular-nums"
      >
        {t("tables.results", { count, total })}
      </span>
    </div>
  );
}

export function TableViewControls<T extends RowData>({
  table,
  view,
  labels,
}: {
  table: Table<T>;
  view: TableView;
  labels: TableColumnLabel[];
}) {
  const { t } = useTranslation();
  const sorting = table.options.state?.sorting ?? [];
  const sortColumns = labels.filter(({ id }) =>
    table.getColumn(id)?.getCanSort(),
  );
  const name = (id: string) =>
    labels.find((column) => column.id === id)?.label ?? id;
  const primarySort = sorting[0];
  const direction = (desc: boolean) =>
    t(desc ? "tables.descending" : "tables.ascending");
  return (
    <div
      data-slot="table-view-controls"
      className="flex flex-wrap items-center gap-1.5"
    >
      <Popover>
        <PopoverTrigger asChild>
          <Button
            variant="soft"
            size="sm"
            data-slot="table-sort-trigger"
            className="h-8 max-w-72 gap-1.5 rounded-lg px-2.5 text-xs"
            disabled={!view.loaded}
            title={
              primarySort
                ? t("tables.sortSummary", {
                    column: name(primarySort.id),
                    direction: direction(primarySort.desc),
                  })
                : undefined
            }
          >
            <LuArrowUpDown className="size-3.5 shrink-0" />
            <span className="truncate">
              {primarySort ? name(primarySort.id) : t("tables.sort")}
            </span>
            {primarySort && (
              <>
                {primarySort.desc ? (
                  <LuArrowDown
                    aria-hidden="true"
                    className="size-3 shrink-0 text-muted-foreground"
                  />
                ) : (
                  <LuArrowUp
                    aria-hidden="true"
                    className="size-3 shrink-0 text-muted-foreground"
                  />
                )}
                <span className="sr-only">{direction(primarySort.desc)}</span>
              </>
            )}
            {sorting.length > 1 && (
              <span className="rounded-full bg-foreground/10 px-1.5 text-[10px] leading-4 tabular-nums">
                +{sorting.length - 1}
              </span>
            )}
          </Button>
        </PopoverTrigger>
        <PopoverContent align="end" className="w-80 space-y-2 p-3">
          <p className="px-1 text-xs font-medium text-muted-foreground">
            {t("tables.sort")}
          </p>
          {sorting.map((sort, index) => (
            <div
              key={sort.id}
              data-sort-index={index}
              className="flex items-center gap-1"
            >
              <span className="w-4 text-center text-xs text-muted-foreground tabular-nums">
                {index + 1}
              </span>
              <Select
                value={sort.id}
                onValueChange={(id) =>
                  table.setSorting(
                    sorting.map((item, i) =>
                      i === index ? { ...item, id } : item,
                    ),
                  )
                }
              >
                <SelectTrigger
                  variant="soft"
                  aria-label={t("tables.sortColumn")}
                  className="h-8 min-w-0 flex-1 text-xs"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {sortColumns
                    .filter(
                      (column) =>
                        column.id === sort.id ||
                        !sorting.some((item) => item.id === column.id),
                    )
                    .map((column) => (
                      <SelectItem key={column.id} value={column.id}>
                        {column.label}
                      </SelectItem>
                    ))}
                </SelectContent>
              </Select>
              <Button
                size="icon"
                variant="soft"
                className="size-8 rounded-lg"
                aria-label={direction(sort.desc)}
                onClick={() =>
                  table.setSorting(
                    sorting.map((item, i) =>
                      i === index ? { ...item, desc: !item.desc } : item,
                    ),
                  )
                }
              >
                {sort.desc ? <LuArrowDown /> : <LuArrowUp />}
              </Button>
              <Button
                size="icon"
                variant="subtle"
                className="size-8 rounded-lg"
                disabled={index === 0}
                aria-label={t("tables.moveSortUp")}
                onClick={() => {
                  const next = [...sorting];
                  [next[index - 1], next[index]] = [
                    next[index],
                    next[index - 1],
                  ];
                  table.setSorting(next);
                }}
              >
                <LuChevronsUp />
              </Button>
              <Button
                size="icon"
                variant="subtle"
                className="size-8 rounded-lg"
                aria-label={t("tables.removeSort")}
                onClick={() =>
                  table.setSorting(sorting.filter((_, i) => i !== index))
                }
              >
                <LuX />
              </Button>
            </div>
          ))}
          <div className="flex gap-1 pt-1">
            <Button
              size="sm"
              variant="soft"
              className="h-8 rounded-lg text-xs"
              disabled={sortColumns.every((column) =>
                sorting.some((sort) => sort.id === column.id),
              )}
              onClick={() => {
                const column = sortColumns.find(
                  (item) => !sorting.some((sort) => sort.id === item.id),
                );
                if (column)
                  table.setSorting([
                    ...sorting,
                    { id: column.id, desc: false },
                  ]);
              }}
            >
              <LuPlus />
              {t("tables.addSort")}
            </Button>
            <Button
              variant="subtle"
              size="sm"
              className="h-8 rounded-lg text-xs"
              onClick={() => table.setSorting([])}
            >
              {t("tables.clearSort")}
            </Button>
          </div>
        </PopoverContent>
      </Popover>
      <Popover>
        <PopoverTrigger asChild>
          <Button
            data-slot="table-view-trigger"
            variant="soft"
            size="sm"
            className="h-8 gap-1.5 rounded-lg px-2.5 text-xs"
            disabled={!view.loaded}
          >
            <LuSettings2 className="size-3.5" />
            {t("tables.view")}
          </Button>
        </PopoverTrigger>
        <PopoverContent align="end" className="w-64 space-y-3 p-3">
          <div className="space-y-1">
            <p className="px-1 text-xs font-medium text-muted-foreground">
              {t("tables.columns")}
            </p>
            <div className="max-h-56 space-y-0.5 overflow-y-auto">
              {labels
                .filter((label) => label.hideable !== false)
                .map((label) => (
                  <label
                    htmlFor={`table-visible-${view.id}-${label.id}`}
                    key={label.id}
                    className="flex min-h-8 cursor-pointer items-center gap-2 rounded-md px-1.5 transition-colors duration-100 hover:bg-foreground/6"
                  >
                    <Checkbox
                      id={`table-visible-${view.id}-${label.id}`}
                      checked={
                        table.getColumn(label.id)?.getIsVisible() ?? false
                      }
                      onCheckedChange={(checked) =>
                        view.setVisibility((prev) => ({
                          ...prev,
                          [label.id]: !!checked,
                        }))
                      }
                    />
                    <span className="text-sm">{label.label}</span>
                  </label>
                ))}
            </div>
          </div>
          <div className="space-y-1">
            <p className="px-1 text-xs font-medium text-muted-foreground">
              {t("tables.rowHeight")}
            </p>
            <Select
              value={view.preferences.density}
              onValueChange={(value: "compact" | "comfortable") =>
                view.setDensity(value)
              }
            >
              <SelectTrigger
                variant="soft"
                aria-label={t("tables.rowHeight")}
                className="h-8 w-full text-xs"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="compact">{t("tables.compact")}</SelectItem>
                <SelectItem value="comfortable">
                  {t("tables.comfortable")}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>
          <Button
            variant="subtle"
            size="sm"
            className="h-8 w-full rounded-lg text-xs"
            onClick={view.reset}
          >
            {t("tables.resetView")}
          </Button>
        </PopoverContent>
      </Popover>
      {view.error && (
        <span
          role="alert"
          className="flex items-center gap-1 text-xs text-destructive-text"
        >
          {view.error}
          <Button
            variant="subtle"
            size="sm"
            className="h-7 rounded-lg text-xs"
            onClick={view.retry}
          >
            {t("common.buttons.retry")}
          </Button>
        </span>
      )}
    </div>
  );
}

/** A column header that sorts its column, shaped like the other headers. */
export function SortableColumnHeader<T extends RowData>({
  column,
  label,
}: {
  column: Column<T, unknown>;
  label: string;
}) {
  const sorted = column.getIsSorted();
  return (
    <Button
      variant="subtle"
      onClick={() => column.toggleSorting(sorted === "asc")}
      className="-ml-1.5 h-6 justify-start gap-1 rounded-md px-1.5 text-left text-xs font-medium"
    >
      {label}
      {sorted === "asc" ? (
        <LuArrowUp className="size-3" />
      ) : sorted === "desc" ? (
        <LuArrowDown className="size-3" />
      ) : (
        <LuArrowUpDown className="size-3 opacity-60" />
      )}
    </Button>
  );
}

const selectionAnchors = new WeakMap<object, string>();

/** Centers a selection checkbox in its cell, at any column width. Inline, it
 * would sit on the text baseline and hug the cell's start. */
export function TableCheckboxSlot({ children }: { children: React.ReactNode }) {
  return <span className="flex items-center justify-center">{children}</span>;
}

export function TableSelectHeader<T extends RowData>({
  table,
}: {
  table: Table<T>;
}) {
  const { t } = useTranslation();
  const selectable = table
    .getRowModel()
    .rows.filter((row) => row.getCanSelect());
  const selected = selectable.filter((row) => row.getIsSelected()).length;
  return (
    <TableCheckboxSlot>
      <Checkbox
        aria-label={t("common.aria.selectAll")}
        className="relative after:absolute after:-inset-2 after:content-['']"
        checked={
          selected > 0 && selected === selectable.length
            ? true
            : selected > 0
              ? "indeterminate"
              : false
        }
        disabled={!selectable.length}
        onCheckedChange={(checked) => {
          selectionAnchors.delete(table);
          table.setRowSelection(
            checked
              ? Object.fromEntries(selectable.map((row) => [row.id, true]))
              : {},
          );
        }}
      />
    </TableCheckboxSlot>
  );
}

export function TableRowCheckbox<T extends RowData>({
  table,
  row,
  icon,
}: {
  table: Table<T>;
  row: Row<T>;
  icon?: React.ReactNode;
}) {
  const { t } = useTranslation();
  const hasSelection = table.getSelectedRowModel().rows.length > 0;
  return (
    <TableCheckboxSlot>
      <span className="group relative flex size-4 shrink-0 items-center justify-center">
        <Checkbox
          aria-label={t("common.aria.selectRow")}
          checked={row.getIsSelected()}
          disabled={!row.getCanSelect()}
          className={cn(
            "relative after:absolute after:-inset-2 after:content-['']",
            icon &&
              !hasSelection &&
              "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
          )}
          onClick={(event) => {
            if (!row.getCanSelect()) return;
            const ids = table
              .getRowModel()
              .rows.filter((item) => item.getCanSelect())
              .map((item) => item.id);
            table.setRowSelection((prev) =>
              selectTableRange(
                ids,
                prev,
                selectionAnchors.get(table) ?? null,
                row.id,
                !row.getIsSelected(),
                event.shiftKey,
              ),
            );
            if (!event.shiftKey) selectionAnchors.set(table, row.id);
          }}
        />
        {icon && !hasSelection && (
          <span
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 group-hover:opacity-0 group-focus-within:opacity-0"
          >
            {icon}
          </span>
        )}
      </span>
    </TableCheckboxSlot>
  );
}

export function TableColumnResizer<T extends RowData>({
  header,
  label,
  disabled,
}: {
  header: Header<T, unknown>;
  label: string;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const beginResize = (
    event: React.MouseEvent<HTMLDivElement> | React.TouchEvent<HTMLDivElement>,
  ) => {
    if (disabled) return;
    const width =
      event.currentTarget.parentElement?.getBoundingClientRect().width;
    if (width)
      flushSync(() =>
        header.getContext().table.setColumnSizing((prev) => ({
          ...prev,
          [header.column.id]: Math.max(48, Math.min(1600, width)),
        })),
      );
    header.getResizeHandler()(event);
  };
  if (!header.column.getCanResize()) return null;
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={t("tables.resizeColumn", { column: label })}
      aria-valuenow={Math.round(header.column.getSize())}
      aria-valuemin={48}
      aria-valuemax={1600}
      tabIndex={disabled ? -1 : 0}
      className="absolute top-1.5 right-0 bottom-1.5 z-20 w-1 cursor-col-resize touch-none rounded-full transition-colors duration-100 hover:bg-foreground/20 focus-visible:bg-primary focus-visible:outline-2 focus-visible:outline-ring after:absolute after:-inset-x-1 after:inset-y-0 after:content-['']"
      onDoubleClick={() => {
        if (!disabled) header.column.resetSize();
      }}
      onMouseDown={beginResize}
      onTouchStart={beginResize}
      onKeyDown={(event) => {
        if (
          disabled ||
          !["ArrowLeft", "ArrowRight", "Home"].includes(event.key)
        )
          return;
        event.preventDefault();
        event.stopPropagation();
        const width =
          event.currentTarget.parentElement?.getBoundingClientRect().width ??
          header.column.getSize();
        const delta = event.key === "ArrowRight" ? 16 : -16;
        if (event.key === "Home") header.column.resetSize();
        else
          header.getContext().table.setColumnSizing((prev) => ({
            ...prev,
            [header.column.id]: Math.max(48, Math.min(1600, width + delta)),
          }));
      }}
    />
  );
}
