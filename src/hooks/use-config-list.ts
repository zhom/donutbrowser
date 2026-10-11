import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";

/** A list the backend owns, loaded by `command` and reloaded on `event`. */
export function useConfigList<T>(command: string, event: string): T[] {
  const [items, setItems] = useState<T[]>([]);

  useEffect(() => {
    let mounted = true;
    let unlisten: (() => void) | undefined;
    const load = async () => {
      try {
        const next = await invoke<T[]>(command);
        if (mounted) setItems(next);
      } catch (error) {
        console.error(`Failed to load ${command}:`, error);
      }
    };
    void load();
    void listen(event, () => {
      void load();
    }).then((stop) => {
      if (mounted) unlisten = stop;
      else stop();
    });
    return () => {
      mounted = false;
      unlisten?.();
    };
  }, [command, event]);

  return items;
}
