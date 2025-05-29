import type { TableSortingSettings } from "@/types";
import type { SortingState } from "@tanstack/react-table";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";

export function useTableSorting() {
  const [sortingSettings, setSortingSettings] = useState<TableSortingSettings>({
    column: "name",
    direction: "asc",
  });
  const [isLoaded, setIsLoaded] = useState(false);

  // Load sorting settings on mount
  useEffect(() => {
    const loadSettings = async () => {
      try {
        const settings = await invoke<TableSortingSettings>(
          "get_table_sorting_settings"
        );
        setSortingSettings(settings);
      } catch (error) {
        console.error("Failed to load table sorting settings:", error);
        // Keep default settings if loading fails
      } finally {
        setIsLoaded(true);
      }
    };

    void loadSettings();
  }, []);

  // Save sorting settings to disk
  const saveSortingSettings = useCallback(
    async (settings: TableSortingSettings) => {
      try {
        await invoke("save_table_sorting_settings", { sorting: settings });
        setSortingSettings(settings);
      } catch (error) {
        console.error("Failed to save table sorting settings:", error);
      }
    },
    []
  );

  // Convert our settings to tanstack table sorting format
  const getTableSorting = useCallback((): SortingState => {
    if (!isLoaded) return [];

    return [
      {
        id: sortingSettings.column,
        desc: sortingSettings.direction === "desc",
      },
    ];
  }, [sortingSettings, isLoaded]);

  // Update sorting when table state changes
  const updateSorting = useCallback(
    (sorting: SortingState) => {
      if (!isLoaded) return;

      if (sorting.length > 0) {
        const newSettings: TableSortingSettings = {
          column: sorting[0].id,
          direction: sorting[0].desc ? "desc" : "asc",
        };
        void saveSortingSettings(newSettings);
      }
    },
    [saveSortingSettings, isLoaded]
  );

  return {
    sortingSettings,
    isLoaded,
    getTableSorting,
    updateSorting,
  };
}
