import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import type { Extension, ExtensionGroup } from "@/types";

export function useExtensionEvents() {
  const [extensions, setExtensions] = useState<Extension[]>([]);
  const [extensionGroups, setExtensionGroups] = useState<ExtensionGroup[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadExtensions = useCallback(async () => {
    try {
      const exts = await invoke<Extension[]>("list_extensions");
      setExtensions(exts);
      setError(null);
    } catch (err: unknown) {
      console.error("Failed to load extensions:", err);
      setExtensions([]);
    }
  }, []);

  const loadExtensionGroups = useCallback(async () => {
    try {
      const groups = await invoke<ExtensionGroup[]>("list_extension_groups");
      setExtensionGroups(groups);
      setError(null);
    } catch (err: unknown) {
      console.error("Failed to load extension groups:", err);
      setExtensionGroups([]);
    }
  }, []);

  const loadAll = useCallback(async () => {
    await Promise.all([loadExtensions(), loadExtensionGroups()]);
  }, [loadExtensions, loadExtensionGroups]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    const setup = async () => {
      try {
        await loadAll();
        unlisten = await listen("extensions-changed", () => {
          void loadAll();
        });
      } catch (err) {
        console.error("Failed to setup extension event listeners:", err);
        setError(
          `Failed to setup extension event listeners: ${JSON.stringify(err)}`,
        );
      } finally {
        setIsLoading(false);
      }
    };

    void setup();

    return () => {
      if (unlisten) unlisten();
    };
  }, [loadAll]);

  return {
    extensions,
    extensionGroups,
    isLoading,
    error,
    loadExtensions,
    loadExtensionGroups,
    loadAll,
  };
}
