"use client";

import { useTranslation } from "react-i18next";

import {
  formatGroupShortcut,
  formatShortcut,
  SHORTCUTS,
  type ShortcutDef,
} from "@/lib/shortcuts";

interface GroupTarget {
  id: string;
  name: string;
}

interface ShortcutsPageProps {
  /** Ordered list — first 9 entries display their Mod+digit binding. */
  groupTargets: GroupTarget[];
}

function Tokens({ tokens }: { tokens: string[] }) {
  return (
    <div className="flex shrink-0 items-center gap-1">
      {tokens.map((tok, i) => (
        <kbd
          key={i}
          className="inline-flex h-6 min-w-6 items-center justify-center rounded border border-border bg-muted px-1.5 text-[11px] font-medium text-foreground"
        >
          {tok}
        </kbd>
      ))}
    </div>
  );
}

function ShortcutTokens({ shortcut }: { shortcut: ShortcutDef }) {
  return <Tokens tokens={formatShortcut(shortcut)} />;
}

export function ShortcutsPage({ groupTargets }: ShortcutsPageProps) {
  const { t } = useTranslation();

  const sections: Array<{ key: ShortcutDef["group"]; titleKey: string }> = [
    { key: "navigation", titleKey: "commandPalette.groups.navigation" },
    { key: "actions", titleKey: "commandPalette.groups.actions" },
  ];

  const digitGroups = groupTargets.slice(0, 9);

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-6 pt-4 pb-8">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-6">
        <header className="flex flex-col gap-1">
          <h1 className="text-lg font-semibold">{t("shortcutsPage.title")}</h1>
          <p className="text-xs text-muted-foreground">
            {t("shortcutsPage.description")}
          </p>
        </header>

        {sections.map(({ key, titleKey }) => {
          const items = SHORTCUTS.filter((s) => s.group === key);
          if (items.length === 0) return null;
          return (
            <section key={key} className="flex flex-col gap-2">
              <h2 className="text-[10px] tracking-wide text-muted-foreground uppercase">
                {t(titleKey)}
              </h2>
              <div className="divide-y divide-border rounded-md border bg-card">
                {items.map((s) => (
                  <div
                    key={s.id}
                    className="flex items-center justify-between gap-4 px-3 py-2"
                  >
                    <span
                      className="min-w-0 truncate text-sm"
                      title={t(s.labelKey)}
                    >
                      {t(s.labelKey)}
                    </span>
                    <ShortcutTokens shortcut={s} />
                  </div>
                ))}
              </div>
            </section>
          );
        })}

        {digitGroups.length > 0 ? (
          <section className="flex flex-col gap-2">
            <h2 className="text-[10px] tracking-wide text-muted-foreground uppercase">
              {t("commandPalette.groups.profileGroups")}
            </h2>
            <div className="divide-y divide-border rounded-md border bg-card">
              {digitGroups.map((target, i) => (
                <div
                  key={target.id}
                  className="flex items-center justify-between gap-4 px-3 py-2"
                >
                  <span
                    className="min-w-0 truncate text-sm"
                    title={target.name}
                  >
                    {target.name}
                  </span>
                  <Tokens tokens={formatGroupShortcut(i + 1)} />
                </div>
              ))}
            </div>
          </section>
        ) : null}
      </div>
    </div>
  );
}
