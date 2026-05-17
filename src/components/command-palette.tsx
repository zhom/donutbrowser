"use client";

import { useTranslation } from "react-i18next";
import { FaDownload } from "react-icons/fa";
import { FiWifi } from "react-icons/fi";
import { GoGear } from "react-icons/go";
import {
  LuCircleStop,
  LuCloud,
  LuInfo,
  LuKeyboard,
  LuPlay,
  LuPlug,
  LuPuzzle,
  LuUser,
  LuUsers,
} from "react-icons/lu";

import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
} from "@/components/ui/command";
import {
  formatGroupShortcut,
  formatShortcut,
  SHORTCUTS,
  type ShortcutDef,
  type ShortcutId,
} from "@/lib/shortcuts";
import type { BrowserProfile } from "@/types";

interface GroupTarget {
  id: string;
  name: string;
}

interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAction: (id: ShortcutId) => void;
  /** Ordered list of groups for Mod+1..9. Index 0 is the catch-all entry. */
  groupTargets: GroupTarget[];
  onSelectGroup: (id: string) => void;
  /** All profiles for launch/stop/info entries. */
  profiles: BrowserProfile[];
  runningProfileIds: Set<string>;
  onLaunchProfile: (profile: BrowserProfile) => void;
  onKillProfile: (profile: BrowserProfile) => void;
  onShowProfileInfo: (profile: BrowserProfile) => void;
}

const ICONS: Record<ShortcutId, React.ComponentType<{ className?: string }>> = {
  openPalette: LuKeyboard,
  openShortcuts: LuKeyboard,
  importProfile: FaDownload,
  goProfiles: LuUser,
  goProxies: FiWifi,
  goExtensions: LuPuzzle,
  goGroups: LuUsers,
  goIntegrations: LuPlug,
  goAccount: LuCloud,
  goSettings: GoGear,
};

function Tokens({ tokens }: { tokens: string[] }) {
  return (
    <CommandShortcut className="flex items-center gap-0.5">
      {tokens.map((tok, i) => (
        <kbd
          key={i}
          className="inline-flex items-center justify-center min-w-[1.25rem] h-5 px-1 rounded border border-border bg-muted text-[10px] font-medium text-muted-foreground"
        >
          {tok}
        </kbd>
      ))}
    </CommandShortcut>
  );
}

function ShortcutTokens({ shortcut }: { shortcut: ShortcutDef }) {
  return <Tokens tokens={formatShortcut(shortcut)} />;
}

/**
 * Token-AND fuzzy filter. Every whitespace-separated token in the query has
 * to appear as a substring somewhere in the item's value or its keywords; the
 * score is reduced when tokens appear later in the haystack so a closer match
 * sorts higher. "ctest info" matches "Info — ctest" — the default cmdk filter
 * requires tokens in document order so it would otherwise return zero.
 */
function fuzzyFilter(
  value: string,
  search: string,
  keywords?: string[],
): number {
  if (!search.trim()) return 1;
  const haystack = [value, ...(keywords ?? [])].join(" ").toLowerCase();
  const tokens = search.toLowerCase().split(/\s+/).filter(Boolean);
  let score = 0;
  for (const tok of tokens) {
    const idx = haystack.indexOf(tok);
    if (idx === -1) return 0;
    score += 1 / (1 + idx);
  }
  return score / tokens.length;
}

export function CommandPalette({
  open,
  onOpenChange,
  onAction,
  groupTargets,
  onSelectGroup,
  profiles,
  runningProfileIds,
  onLaunchProfile,
  onKillProfile,
  onShowProfileInfo,
}: CommandPaletteProps) {
  const { t } = useTranslation();

  // `cmdk` calls onSelect BEFORE the dialog closes. Close first, then dispatch
  // on the next tick so an action that opens another dialog doesn't race
  // this one's close animation.
  const dispatch = (fn: () => void) => {
    onOpenChange(false);
    setTimeout(fn, 0);
  };

  const byGroup = (group: ShortcutDef["group"]) =>
    SHORTCUTS.filter((s) => s.group === group);

  // Limit to 9 — only the first 9 group targets have a Mod+digit binding.
  // We still display more in the palette (without a shortcut hint) so the
  // user can search/jump to any of them.
  const renderGroup = (target: GroupTarget, index: number) => (
    <CommandItem
      key={target.id}
      onSelect={() => {
        dispatch(() => {
          onSelectGroup(target.id);
        });
      }}
    >
      <LuUsers />
      <span>{target.name}</span>
      {index < 9 ? <Tokens tokens={formatGroupShortcut(index + 1)} /> : null}
    </CommandItem>
  );

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange} filter={fuzzyFilter}>
      <CommandInput placeholder={t("commandPalette.placeholder")} />
      <CommandList>
        <CommandEmpty>{t("commandPalette.empty")}</CommandEmpty>

        <CommandGroup heading={t("commandPalette.groups.navigation")}>
          {byGroup("navigation").map((s) => {
            const Icon = ICONS[s.id];
            return (
              <CommandItem
                key={s.id}
                onSelect={() => {
                  dispatch(() => {
                    onAction(s.id);
                  });
                }}
              >
                <Icon />
                <span>{t(s.labelKey)}</span>
                <ShortcutTokens shortcut={s} />
              </CommandItem>
            );
          })}
        </CommandGroup>

        {groupTargets.length > 0 ? (
          <>
            <CommandSeparator />
            <CommandGroup heading={t("commandPalette.groups.profileGroups")}>
              {groupTargets.map((target, i) => renderGroup(target, i))}
            </CommandGroup>
          </>
        ) : null}

        {profiles.length > 0 ? (
          <>
            <CommandSeparator />
            <CommandGroup heading={t("commandPalette.groups.profiles")}>
              {profiles.map((p) => {
                const running = runningProfileIds.has(p.id);
                return running ? (
                  <CommandItem
                    key={`run-${p.id}`}
                    onSelect={() => {
                      dispatch(() => {
                        onKillProfile(p);
                      });
                    }}
                  >
                    <LuCircleStop />
                    <span>
                      {t("commandPalette.actions.stopProfile", {
                        name: p.name,
                      })}
                    </span>
                  </CommandItem>
                ) : (
                  <CommandItem
                    key={`run-${p.id}`}
                    onSelect={() => {
                      dispatch(() => {
                        onLaunchProfile(p);
                      });
                    }}
                  >
                    <LuPlay />
                    <span>
                      {t("commandPalette.actions.launchProfile", {
                        name: p.name,
                      })}
                    </span>
                  </CommandItem>
                );
              })}
              {profiles.map((p) => (
                <CommandItem
                  key={`info-${p.id}`}
                  onSelect={() => {
                    dispatch(() => {
                      onShowProfileInfo(p);
                    });
                  }}
                >
                  <LuInfo />
                  <span>
                    {t("commandPalette.actions.profileInfo", { name: p.name })}
                  </span>
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        ) : null}

        <CommandSeparator />

        <CommandGroup heading={t("commandPalette.groups.actions")}>
          {byGroup("actions").map((s) => {
            const Icon = ICONS[s.id];
            return (
              <CommandItem
                key={s.id}
                onSelect={() => {
                  dispatch(() => {
                    onAction(s.id);
                  });
                }}
              >
                <Icon />
                <span>{t(s.labelKey)}</span>
                <ShortcutTokens shortcut={s} />
              </CommandItem>
            );
          })}
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
