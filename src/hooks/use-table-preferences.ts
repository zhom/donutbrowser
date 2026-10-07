"use client";

import type {
  ColumnSizingState,
  ColumnVisibilityState,
  SortingState,
  Updater,
} from "@tanstack/react-table";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { translateBackendError } from "@/lib/backend-errors";
import type { TableId, TablePreferences } from "@/lib/table-tools";

const writes = new Map<TableId, Promise<unknown>>();

function save(id: TableId, preferences: TablePreferences) {
  const previous = writes.get(id) ?? Promise.resolve();
  const next = previous
    .catch(() => {})
    .then(() => invoke("save_table_preferences", { tableId: id, preferences }));
  writes.set(id, next);
  return next;
}

export function useTablePreferences(id: TableId) {
  const { t } = useTranslation();
  const [preferences, setPreferences] = useState<TablePreferences>({
    visibility: {},
    sizing: {},
    sorting: [{ id: "name", desc: false }],
    density: "compact",
  });
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const dirty = useRef(false);
  const latest = useRef(preferences);
  latest.current = preferences;

  useEffect(() => {
    let active = true;
    if (loaded) return;
    if (attempt > 0) setError(null);
    void (writes.get(id) ?? Promise.resolve())
      .catch(() => {})
      .then(() =>
        invoke<TablePreferences>("get_table_preferences", { tableId: id }),
      )
      .then((value) => {
        if (!active) return;
        setPreferences(value);
        setLoaded(true);
        setError(null);
      })
      .catch((err: unknown) => {
        if (active) setError(translateBackendError(t, err));
      });
    return () => {
      active = false;
    };
  }, [id, loaded, attempt, t]);

  useEffect(() => {
    if (!loaded || !dirty.current) return;
    if (attempt > 0) setError(null);
    let active = true;
    const timer = setTimeout(() => {
      void save(id, preferences)
        .then(() => {
          if (latest.current === preferences) dirty.current = false;
          if (active) setError(null);
        })
        .catch((err: unknown) => {
          if (active) setError(translateBackendError(t, err));
        });
    }, 180);
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [id, loaded, preferences, attempt, t]);

  useEffect(
    () => () => {
      if (dirty.current)
        void save(id, latest.current).catch((err: unknown) =>
          console.error("Table preferences save failed", err),
        );
    },
    [id],
  );

  const update = useCallback(
    <K extends keyof TablePreferences>(
      key: K,
      value: Updater<TablePreferences[K]>,
    ) => {
      if (!loaded) return;
      dirty.current = true;
      setPreferences((prev) => ({
        ...prev,
        [key]: typeof value === "function" ? value(prev[key]) : value,
      }));
    },
    [loaded],
  );

  return {
    id,
    preferences,
    loaded,
    error,
    retry: () => setAttempt((value) => value + 1),
    setSorting: useCallback(
      (value: Updater<SortingState>) => update("sorting", value),
      [update],
    ),
    setVisibility: useCallback(
      (value: Updater<ColumnVisibilityState>) => update("visibility", value),
      [update],
    ),
    setSizing: useCallback(
      (value: Updater<ColumnSizingState>) => update("sizing", value),
      [update],
    ),
    setDensity: (value: TablePreferences["density"]) =>
      update("density", value),
    reset: () => {
      if (!loaded) return;
      dirty.current = true;
      setPreferences({
        visibility: {},
        sizing: {},
        sorting: [{ id: "name", desc: false }],
        density: "compact",
      });
    },
  };
}

export type TableView = ReturnType<typeof useTablePreferences>;
