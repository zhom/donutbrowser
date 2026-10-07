"use client";

import type { RowData } from "@tanstack/react-table";
import type { LegacyTable as Table } from "@tanstack/react-table/legacy";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  filterTableRows,
  type TableFilter,
  type TableFilterValues,
} from "@/lib/table-tools";

export function useTableFilters<T>(
  data: T[],
  searchText: (row: T) => string,
  definitions: TableFilter<T>[],
  clearSelection: () => void,
) {
  const [query, setQuery] = useState("");
  const [values, setValues] = useState<TableFilterValues>({});
  const rows = useMemo(
    () => filterTableRows(data, query, searchText, definitions, values),
    [data, query, searchText, definitions, values],
  );
  const facets = useMemo(
    () =>
      definitions.map((filter) => {
        const candidates = filterTableRows(
          data,
          query,
          searchText,
          definitions,
          values,
          filter.id,
        );
        const counts = new Map<string, number>();
        for (const row of candidates) {
          const raw = filter.value(row);
          for (const value of new Set(Array.isArray(raw) ? raw : [raw]))
            counts.set(value, (counts.get(value) ?? 0) + 1);
        }
        return {
          id: filter.id,
          label: filter.label,
          options: filter.options.map((option) => ({
            ...option,
            count: counts.get(option.value) ?? 0,
          })),
        };
      }),
    [data, query, searchText, definitions, values],
  );
  return {
    query,
    values,
    rows,
    facets,
    total: data.length,
    onQueryChange: (value: string) => {
      clearSelection();
      setQuery(value);
    },
    onValuesChange: (value: TableFilterValues) => {
      clearSelection();
      setValues(value);
    },
    reset: () => {
      clearSelection();
      setQuery("");
      setValues({});
    },
  };
}

export function useTableKeyboard<T extends RowData>(
  table: Table<T>,
  options?: {
    scrollToIndex?: (index: number) => void;
    edit?: (rowId: string, columnId: string) => boolean;
  },
) {
  const tableRef = useRef<HTMLTableElement>(null);
  const [active, setActive] = useState<{
    rowId: string;
    columnId: string;
  } | null>(null);
  const pending = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (pending.current !== null) cancelAnimationFrame(pending.current);
    },
    [],
  );
  const rows = table.getRowModel().rows;
  const columns = table.getVisibleLeafColumns();
  const rowId = rows.some((row) => row.id === active?.rowId)
    ? active?.rowId
    : rows[0]?.id;
  const columnId = columns.some((column) => column.id === active?.columnId)
    ? active?.columnId
    : (columns.find((column) => column.id === "name")?.id ?? columns[0]?.id);

  const focusCell = useCallback((nextRow: string, nextColumn: string) => {
    setActive({ rowId: nextRow, columnId: nextColumn });
    if (pending.current !== null) cancelAnimationFrame(pending.current);
    let attempts = 0;
    const focus = () => {
      const element = tableRef.current?.querySelector<HTMLElement>(
        `[data-table-row="${CSS.escape(nextRow)}"] [data-table-column="${CSS.escape(nextColumn)}"]`,
      );
      if (element) {
        element.focus({ preventScroll: true });
        element.scrollIntoView({ block: "nearest", inline: "nearest" });
        pending.current = null;
      } else if (++attempts < 8) pending.current = requestAnimationFrame(focus);
    };
    pending.current = requestAnimationFrame(focus);
  }, []);

  return {
    tableRef,
    cellProps: (currentRow: string, currentColumn: string) => ({
      "data-table-column": currentColumn,
      tabIndex: currentRow === rowId && currentColumn === columnId ? 0 : -1,
      onFocus: () =>
        setActive((prev) =>
          prev?.rowId === currentRow && prev.columnId === currentColumn
            ? prev
            : { rowId: currentRow, columnId: currentColumn },
        ),
    }),
    onKeyDown: (event: React.KeyboardEvent<HTMLTableElement>) => {
      if (
        event.defaultPrevented ||
        event.altKey ||
        event.metaKey ||
        event.ctrlKey
      )
        return;
      const target = event.target as HTMLElement;
      const cell = target.closest<HTMLElement>("[data-table-column]");
      if (!cell || !tableRef.current?.contains(cell)) return;
      if (target !== cell) return;
      const currentRow =
        cell.closest<HTMLElement>("[data-table-row]")?.dataset.tableRow;
      const currentColumn = cell.dataset.tableColumn;
      let r = rows.findIndex((row) => row.id === currentRow);
      let c = columns.findIndex((column) => column.id === currentColumn);
      if (r < 0 || c < 0) return;
      if (event.key === "Enter" || event.key === "F2") {
        event.preventDefault();
        if (
          !(
            currentRow &&
            currentColumn &&
            options?.edit?.(currentRow, currentColumn)
          )
        ) {
          const control = cell.querySelector<HTMLElement>(
            'button:not([disabled]), input:not([disabled]), [role="combobox"]',
          );
          control?.focus();
          if (
            event.key === "Enter" ||
            ["name", "tags", "note"].includes(currentColumn ?? "")
          )
            control?.click();
        }
        return;
      }
      if (event.key === "ArrowDown") r++;
      else if (event.key === "ArrowUp") r--;
      else if (event.key === "ArrowRight") c++;
      else if (event.key === "ArrowLeft") c--;
      else if (event.key === "Home") c = 0;
      else if (event.key === "End") c = columns.length - 1;
      else if (event.key === "Tab") {
        c += event.shiftKey ? -1 : 1;
        if (c < 0) {
          r--;
          c = columns.length - 1;
        }
        if (c >= columns.length) {
          r++;
          c = 0;
        }
        if (r < 0 || r >= rows.length) return;
      } else return;
      event.preventDefault();
      r = Math.max(0, Math.min(rows.length - 1, r));
      c = Math.max(0, Math.min(columns.length - 1, c));
      options?.scrollToIndex?.(r);
      focusCell(rows[r].id, columns[c].id);
    },
    onKeyDownCapture: (event: React.KeyboardEvent<HTMLTableElement>) => {
      if (event.key !== "Escape" || !rowId || !columnId) return;
      const target = event.target as HTMLElement;
      if (target.matches('input,textarea,[contenteditable="true"]'))
        focusCell(rowId, columnId);
    },
  };
}
