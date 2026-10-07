"use client";

import { flexRender, type RowSelectionState } from "@tanstack/react-table";
import type {
  LegacyColumnDef as ColumnDef,
  LegacyTable as Table,
} from "@tanstack/react-table/legacy";
import {
  type Dispatch,
  type SetStateAction,
  useCallback,
  useLayoutEffect,
  useMemo,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import {
  DATA_TABLE_CLASSES,
  TableColumnResizer,
  TableFilterControls,
  TableRowCheckbox,
  TableSelectHeader,
  TableViewControls,
} from "@/components/table-controls";
import { Button } from "@/components/ui/button";
import { FadingScrollArea } from "@/components/ui/fading-scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import {
  TableBody,
  TableCell,
  Table as TableElement,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTablePreferences } from "@/hooks/use-table-preferences";
import { useTableFilters, useTableKeyboard } from "@/hooks/use-table-tools";
import {
  resolveTableVisibility,
  type TableFilter,
  type TableId,
} from "@/lib/table-tools";
import { cn } from "@/lib/utils";
import type {
  Extension,
  ExtensionGroup,
  GroupWithCount,
  StoredProxy,
  VpnConfig,
} from "@/types";

type Entity =
  | Extension
  | ExtensionGroup
  | GroupWithCount
  | StoredProxy
  | VpnConfig;
type EntityTableId = Exclude<TableId, "profiles">;
const LABELS: Record<EntityTableId, Record<string, string>> = {
  proxies: {
    name: "common.labels.name",
    protocol: "proxies.management.protocolCol",
    hostPort: "proxies.management.hostPort",
    udp: "proxies.management.udpCol",
    usage: "proxies.management.usage",
    sync: "proxies.management.syncCol",
  },
  vpns: {
    name: "common.labels.name",
    type: "common.labels.type",
    usage: "proxies.management.usage",
    sync: "proxies.management.syncCol",
  },
  extensions: {
    name: "common.labels.name",
    compat: "extensions.compatibility.label",
    source: "appFeedback.source",
    usage: "profiles.title",
    sync: "proxies.management.syncCol",
  },
  extensionGroups: {
    name: "common.labels.name",
    extensions: "extensions.extensionsTab",
    usage: "profiles.title",
    sync: "proxies.management.syncCol",
  },
  groups: {
    name: "common.labels.name",
    count: "groupManagement.profilesCol",
    sync: "proxies.management.syncCol",
  },
};

function entitySearch(row: Entity) {
  const parts = [row.name];
  if ("proxy_settings" in row)
    parts.push(row.proxy_settings.host, row.proxy_settings.proxy_type);
  if ("file_name" in row) parts.push(row.file_name, row.manifest_name ?? "");
  return parts.join(" ");
}

function extraValue(row: Entity, id: string): string | number {
  if (id === "sync") return row.sync_enabled ? 1 : 0;
  if (id === "count" && "count" in row) return row.count;
  if (id === "extensions" && "extension_ids" in row)
    return row.extension_ids.length;
  if (id === "type" && "vpn_type" in row) return row.vpn_type;
  if (id === "source" && "source_kind" in row)
    return row.linked_path ? "linked" : row.source_kind;
  if (id === "compat" && "browser_compatibility" in row)
    return row.browser_compatibility.join(", ");
  if ("proxy_settings" in row) {
    if (id === "protocol") return row.proxy_settings.proxy_type;
    if (id === "hostPort")
      return `${row.proxy_settings.host}:${row.proxy_settings.port}`;
  }
  return "";
}

export function configureEntityColumns<T extends Entity>(
  columns: ColumnDef<T>[],
): ColumnDef<T>[] {
  return columns.map<ColumnDef<T>>((column) => {
    const id =
      column.id ?? ("accessorKey" in column ? String(column.accessorKey) : "");
    if (id === "select")
      return {
        ...column,
        id,
        enableHiding: false,
        enableResizing: false,
        header: ({ table }) => <TableSelectHeader table={table} />,
        cell: ({ row, table }) => <TableRowCheckbox row={row} table={table} />,
      };
    if (id === "actions" || id === "icon")
      return { ...column, enableHiding: false, enableResizing: false };
    const sortable = [
      "sync",
      "count",
      "extensions",
      "type",
      "source",
      "compat",
      "protocol",
      "hostPort",
    ].includes(id);
    return {
      ...column,
      minSize: 48,
      maxSize: 1600,
      enableResizing: true,
      enableHiding: id !== "name",
      ...(sortable
        ? { accessorFn: (row: T) => extraValue(row, id), enableSorting: true }
        : {}),
    };
  });
}

export function useEntityTableTools<T extends Entity>(
  id: EntityTableId,
  data: T[],
  setSelection: Dispatch<SetStateAction<RowSelectionState>>,
) {
  const { t } = useTranslation();
  const view = useTablePreferences(id);
  const [width, setWidth] = useState(1024);
  const labels = useMemo(
    () =>
      Object.entries(LABELS[id]).map(([columnId, key]) => ({
        id: columnId,
        label: t(key),
        hideable: columnId !== "name",
      })),
    [id, t],
  );
  const definitions = useMemo<TableFilter<T>[]>(() => {
    const result: TableFilter<T>[] = [
      {
        id: "sync",
        label: t("proxies.management.syncCol"),
        value: (row) => (row.sync_enabled ? "enabled" : "disabled"),
        options: [
          { value: "enabled", label: t("tables.enabled") },
          { value: "disabled", label: t("tables.disabled") },
        ],
      },
    ];
    const extra =
      id === "proxies"
        ? "protocol"
        : id === "vpns"
          ? "type"
          : id === "extensions"
            ? "source"
            : null;
    if (extra) {
      const values = [
        ...new Set(data.map((row) => String(extraValue(row, extra)))),
      ].sort();
      result.push({
        id: extra,
        label: t(LABELS[id][extra]),
        value: (row) => String(extraValue(row, extra)),
        options: values.map((value) => ({
          value,
          label:
            extra === "source"
              ? t(
                  value === "linked"
                    ? "extensions.source.linked"
                    : value === "unpacked"
                      ? "extensions.source.unpacked"
                      : "extensions.source.archive",
                )
              : value.toUpperCase(),
        })),
      });
    }
    return result;
  }, [id, data, t]);
  const clearSelection = useCallback(() => setSelection({}), [setSelection]);
  const filters = useTableFilters(
    data,
    entitySearch,
    definitions,
    clearSelection,
  );
  useLayoutEffect(() => {
    const visible = new Set(filters.rows.map((row) => row.id));
    setSelection((prev) => {
      const ids = Object.keys(prev).filter((id) => prev[id]);
      return ids.every((id) => visible.has(id))
        ? prev
        : Object.fromEntries(
            ids.filter((id) => visible.has(id)).map((id) => [id, true]),
          );
    });
  }, [filters.rows, setSelection]);
  const visibility = resolveTableVisibility(
    id === "proxies"
      ? { hostPort: width >= 672, protocol: width >= 672 }
      : id === "vpns"
        ? { type: width >= 672 }
        : {},
    view.preferences.visibility,
    ["name", "select", "actions", "icon"],
  );
  return { view, labels, filters, visibility, setWidth };
}

export function ManagedDataTable<T extends Entity>({
  table,
  tools,
  loading,
  emptyText,
}: {
  table: Table<T>;
  tools: ReturnType<typeof useEntityTableTools<T>>;
  loading: boolean;
  emptyText: string;
}) {
  const { t } = useTranslation();
  const { view, labels, filters, setWidth } = tools;
  const keyboard = useTableKeyboard(table);
  const resizeRef = useCallback(
    (element: HTMLDivElement | null) => {
      if (!element) return;
      const observer = new ResizeObserver(() => setWidth(element.clientWidth));
      setWidth(element.clientWidth);
      observer.observe(element);
      return () => observer.disconnect();
    },
    [setWidth],
  );
  const rows = table.getRowModel().rows;
  const rowHeight = view.preferences.density === "comfortable" ? 48 : 36;
  const sized =
    Object.keys(view.preferences.sizing).length > 0 ||
    Object.values(view.preferences.visibility).some(Boolean);
  const minWidth = sized
    ? table
        .getVisibleLeafColumns()
        .reduce(
          (sum, column) =>
            sum +
            (column.id === "name" && !view.preferences.sizing.name
              ? 220
              : column.getSize()),
          0,
        )
    : undefined;
  return (
    <div
      ref={resizeRef}
      data-table-id={view.id}
      className="flex min-h-0 flex-1 flex-col gap-3"
    >
      <div className="flex flex-wrap items-start justify-between gap-2">
        <TableFilterControls
          {...filters}
          count={rows.length}
          onReset={filters.reset}
        />
        <TableViewControls table={table} view={view} labels={labels} />
      </div>
      <FadingScrollArea
        className={cn(
          "min-h-0 flex-1",
          table.getSelectedRowModel().rows.length > 0 &&
            "pb-[calc(var(--table-action-bar-height,56px)+32px)]",
        )}
        style={{ "--scroll-fade-top-offset": "32px" } as React.CSSProperties}
      >
        <TableElement
          ref={keyboard.tableRef}
          role="grid"
          aria-label={t("tables.dataTable")}
          aria-rowcount={rows.length + 1}
          aria-colcount={table.getVisibleLeafColumns().length}
          onKeyDown={keyboard.onKeyDown}
          onKeyDownCapture={keyboard.onKeyDownCapture}
          className={cn(DATA_TABLE_CLASSES.table, "w-full")}
          style={{ minWidth, width: minWidth }}
          containerClassName="overflow-visible"
        >
          <TableHeader className={DATA_TABLE_CLASSES.header}>
            {table.getHeaderGroups().map((group) => (
              <TableRow
                key={group.id}
                role="row"
                aria-rowindex={1}
                className={DATA_TABLE_CLASSES.headerRow}
              >
                {group.headers.map((header, index) => (
                  <TableHead
                    key={header.id}
                    role="columnheader"
                    aria-colindex={index + 1}
                    aria-sort={
                      header.column.getIsSorted() === "asc"
                        ? "ascending"
                        : header.column.getIsSorted() === "desc"
                          ? "descending"
                          : undefined
                    }
                    className={DATA_TABLE_CLASSES.head}
                    style={{
                      width:
                        header.column.id === "name" &&
                        !view.preferences.sizing.name
                          ? undefined
                          : header.column.getSize(),
                    }}
                  >
                    {flexRender(
                      header.column.columnDef.header,
                      header.getContext(),
                    )}
                    <TableColumnResizer
                      header={header}
                      label={
                        labels.find((column) => column.id === header.column.id)
                          ?.label ?? ""
                      }
                      disabled={!view.loaded}
                    />
                  </TableHead>
                ))}
              </TableRow>
            ))}
          </TableHeader>
          <TableBody>
            {loading ? (
              Array.from({ length: 5 }, (_, index) => (
                <TableRow
                  key={`loading-${index}`}
                  className={DATA_TABLE_CLASSES.row}
                >
                  <TableCell
                    colSpan={table.getVisibleLeafColumns().length}
                    style={{ height: rowHeight }}
                  >
                    <Skeleton className="h-3 w-2/3" />
                  </TableCell>
                </TableRow>
              ))
            ) : rows.length === 0 ? (
              <TableRow className={DATA_TABLE_CLASSES.row}>
                <TableCell
                  colSpan={table.getVisibleLeafColumns().length}
                  className="py-12 text-center text-muted-foreground"
                >
                  <p>{filters.total ? t("tables.noResults") : emptyText}</p>
                  {filters.total > 0 && (
                    <Button
                      variant="soft"
                      size="sm"
                      className="mt-3 rounded-lg"
                      onClick={filters.reset}
                    >
                      {t("tables.clearFilters")}
                    </Button>
                  )}
                </TableCell>
              </TableRow>
            ) : (
              rows.map((row, index) => (
                <TableRow
                  key={row.id}
                  role="row"
                  aria-rowindex={index + 2}
                  aria-selected={row.getIsSelected()}
                  data-table-row={row.id}
                  data-state={row.getIsSelected() && "selected"}
                  className={DATA_TABLE_CLASSES.row}
                  style={{ height: rowHeight }}
                >
                  {row.getVisibleCells().map((cell, columnIndex) => (
                    <TableCell
                      key={cell.id}
                      role="gridcell"
                      aria-colindex={columnIndex + 1}
                      {...keyboard.cellProps(row.id, cell.column.id)}
                      className={cn(
                        DATA_TABLE_CLASSES.cell,
                        "relative max-w-0",
                      )}
                      style={{
                        width:
                          cell.column.id === "name" &&
                          !view.preferences.sizing.name
                            ? undefined
                            : cell.column.getSize(),
                      }}
                    >
                      {flexRender(
                        cell.column.columnDef.cell,
                        cell.getContext(),
                      )}
                    </TableCell>
                  ))}
                </TableRow>
              ))
            )}
          </TableBody>
        </TableElement>
      </FadingScrollArea>
    </div>
  );
}
