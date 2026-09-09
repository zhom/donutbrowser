"use client";

import { isMacOS } from "@/lib/platform";
import type { TipId } from "@/lib/tips";
import {
  ApiScene,
  ConsistencyScene,
  DnsScene,
  ExtensionsScene,
  GroupsScene,
  ImportScene,
  LinkRouteScene,
  LockScene,
  PaletteScene,
  ProxyRouteScene,
  SweepScene,
  SyncScene,
  TrashScene,
} from "./scenes-essentials";
import {
  AgentScene,
  CloudSyncScene,
  CookieBotScene,
  CrossOsScene,
  RemoteScene,
  TeamScene,
} from "./scenes-plan";

/** The drawing for one tip. Every tip id has one; the switch is exhaustive. */
export function TipScene({ id }: { id: TipId }) {
  switch (id) {
    case "dnsBlocklist":
      return <DnsScene />;
    case "proxyCheck":
      return <ProxyRouteScene />;
    case "groups":
      return <GroupsScene />;
    case "commandPalette":
      return <PaletteScene modLabel={isMacOS() ? "⌘" : "Ctrl"} />;
    case "fingerprintGate":
      return <ConsistencyScene />;
    case "profilePassword":
      return <LockScene />;
    case "clearOnClose":
      return <SweepScene />;
    case "defaultBrowser":
      return <LinkRouteScene />;
    case "extensionGroups":
      return <ExtensionsScene />;
    case "selfHostedSync":
      return <SyncScene />;
    case "trash":
      return <TrashScene />;
    case "localApi":
      return <ApiScene variant="api" />;
    case "importProfiles":
      return <ImportScene />;
    case "cloudBackup":
      return <CloudSyncScene />;
    case "cookieBot":
      return <CookieBotScene />;
    case "crossOs":
      return <CrossOsScene />;
    case "automation":
      return <ApiScene variant="run" />;
    case "agent":
      return <AgentScene />;
    case "team":
      return <TeamScene />;
    case "remoteControl":
      return <RemoteScene />;
  }
}
