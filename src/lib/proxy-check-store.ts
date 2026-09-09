import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useSyncExternalStore } from "react";
import type { ProxyCheckResult, StoredProxy } from "@/types";

/**
 * One receipt per stored proxy, shared by every button that shows it.
 *
 * The profile table renders a check button per row, so two profiles on the
 * same proxy are two buttons. A check run from one row is a fact about the
 * proxy, not the row, and the other row must show it too; the backend cache
 * is only re-read on mount, so it cannot carry that update by itself.
 */
export interface ProxyCheckEntry {
  /** The route the receipt and the in-flight check belong to. */
  routeKey: string;
  result: ProxyCheckResult | null;
  checking: boolean;
  /** Translated failure text from the last check on this route. */
  failure: string | null;
}

const IDLE: ProxyCheckEntry = {
  routeKey: "",
  result: null,
  checking: false,
  failure: null,
};

const entries = new Map<string, ProxyCheckEntry>();
const listeners = new Map<string, Set<() => void>>();
/** Cache reads in flight, keyed by route, so N rows cost one invoke. */
const cacheReads = new Map<string, Promise<void>>();

/** Credentials and the endpoint decide whether an old receipt still applies. */
export function proxyRouteKey(proxy: StoredProxy): string {
  return JSON.stringify([proxy.id, proxy.proxy_settings]);
}

function publish(proxyId: string, next: ProxyCheckEntry) {
  entries.set(proxyId, next);
  for (const listener of listeners.get(proxyId) ?? []) listener();
}

function entryFor(proxyId: string, routeKey: string): ProxyCheckEntry {
  const current = entries.get(proxyId) ?? IDLE;
  return current.routeKey === routeKey ? current : { ...IDLE, routeKey };
}

function subscribe(proxyId: string, listener: () => void) {
  const set = listeners.get(proxyId) ?? new Set();
  set.add(listener);
  listeners.set(proxyId, set);
  return () => {
    set.delete(listener);
    if (set.size === 0) listeners.delete(proxyId);
  };
}

async function loadCached(proxyId: string, routeKey: string) {
  const known = entries.get(proxyId);
  if (known?.routeKey === routeKey) return;
  const pending = cacheReads.get(routeKey);
  if (pending) return pending;
  // The backend keys its cache by the settings it was checked with, so a
  // receipt for a rotated password or a new host never comes back here.
  const read = invoke<ProxyCheckResult | null>("get_cached_proxy_check", {
    proxyId,
  })
    .then((result) => {
      const current = entries.get(proxyId);
      // A check that started while this read was in flight owns the entry.
      if (current?.routeKey === routeKey) return;
      publish(proxyId, { ...IDLE, routeKey, result });
    })
    .catch(() => {
      if (!entries.has(proxyId)) publish(proxyId, { ...IDLE, routeKey });
    })
    .finally(() => {
      cacheReads.delete(routeKey);
    });
  cacheReads.set(routeKey, read);
  return read;
}

/**
 * Run the real check for a proxy and publish the outcome to every subscriber.
 * Resolves with the result, with `null` when a check for this proxy is
 * already running, or rejects with the raw backend error so the caller can
 * translate and announce it; the store keeps the translated text the caller
 * hands back through `failure`.
 */
export async function runProxyCheck(
  proxy: StoredProxy,
  translateFailure: (error: unknown) => string,
): Promise<ProxyCheckResult | null> {
  const routeKey = proxyRouteKey(proxy);
  const before = entryFor(proxy.id, routeKey);
  if (before.checking) return null;
  publish(proxy.id, { ...before, checking: true, failure: null });
  try {
    const result = await invoke<ProxyCheckResult>("check_proxy_validity", {
      proxyId: proxy.id,
      proxySettings: proxy.proxy_settings,
    });
    publish(proxy.id, { routeKey, result, checking: false, failure: null });
    return result;
  } catch (error) {
    publish(proxy.id, {
      routeKey,
      result: {
        ip: "",
        timestamp: Math.floor(Date.now() / 1000),
        is_valid: false,
      },
      checking: false,
      failure: translateFailure(error),
    });
    throw error;
  }
}

export function useProxyCheck(proxy: StoredProxy) {
  const routeKey = proxyRouteKey(proxy);
  const proxyId = proxy.id;
  const subscribeToProxy = useCallback(
    (listener: () => void) => subscribe(proxyId, listener),
    [proxyId],
  );
  const getSnapshot = useCallback(
    () => entries.get(proxyId) ?? IDLE,
    [proxyId],
  );
  const raw = useSyncExternalStore(subscribeToProxy, getSnapshot, getSnapshot);
  // A receipt for a different route (rotated password, new host) is stale
  // and reads as "not checked" until the cache for this route is loaded.
  const entry = raw.routeKey === routeKey ? raw : IDLE;

  useEffect(() => {
    void loadCached(proxyId, routeKey);
  }, [proxyId, routeKey]);

  return entry;
}
