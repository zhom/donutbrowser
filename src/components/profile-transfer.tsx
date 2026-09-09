"use client";

import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { LoadingButton } from "@/components/loading-button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { translateBackendError } from "@/lib/backend-errors";
import { showToast } from "@/lib/toast-utils";
import type { BrowserProfile } from "@/types";

/** The file extension a Donut profile archive carries. */
export const PROFILE_ARCHIVE_EXTENSION = "donutprofile";

interface PortableManifest {
  format_version: number;
  exported_by: string;
  exported_at: number;
  profile_name: string;
  browser: string;
  version: string;
  includes_data: boolean;
  data_omitted_reason?: string;
}

/**
 * Write this profile to a file another machine can read.
 *
 * The data is opt-in rather than assumed: a configuration-only archive is
 * small enough to send, while one carrying cookies and logins is the profile
 * itself and should be moved deliberately.
 */
export function ExportProfileSection({ profile }: { profile: BrowserProfile }) {
  const { t } = useTranslation();
  const [includeData, setIncludeData] = useState(true);
  const [isExporting, setIsExporting] = useState(false);
  const isRunning = Boolean(profile.process_id);

  const handleExport = async () => {
    const destination = await save({
      title: t("profileTransfer.exportTitle"),
      defaultPath: `${profile.name}.${PROFILE_ARCHIVE_EXTENSION}`,
      filters: [
        {
          name: t("profileTransfer.archiveFilter"),
          extensions: [PROFILE_ARCHIVE_EXTENSION],
        },
      ],
    });
    if (!destination) return;

    setIsExporting(true);
    try {
      const manifest = await invoke<PortableManifest>("export_profile", {
        profileId: profile.id,
        destination,
        includeData: includeData && !profile.password_protected,
      });
      if (manifest.includes_data) {
        showToast({
          type: "success",
          title: t("profileTransfer.exportedWithData"),
        });
      } else {
        showToast({
          type: "success",
          title: t("profileTransfer.exportedConfigOnly"),
        });
      }
    } catch (error) {
      showToast({
        type: "error",
        title: translateBackendError(t, error),
      });
    } finally {
      setIsExporting(false);
    }
  };

  return (
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">
        {t("profileTransfer.exportDescription")}
      </p>
      <div className="flex items-center gap-x-2">
        <Checkbox
          id="export-include-data"
          checked={includeData && !profile.password_protected}
          disabled={profile.password_protected || isExporting}
          onCheckedChange={(checked) => {
            setIncludeData(checked === true);
          }}
        />
        <Label htmlFor="export-include-data">
          {t("profileTransfer.includeData")}
        </Label>
      </div>
      {profile.password_protected && (
        <p className="text-sm text-muted-foreground">
          {t("profileTransfer.protectedNotice")}
        </p>
      )}
      {isRunning && (
        <p className="text-sm text-warning-text">
          {t("profileTransfer.runningNotice")}
        </p>
      )}
      <LoadingButton
        isLoading={isExporting}
        disabled={isRunning}
        onClick={() => {
          void handleExport();
        }}
      >
        {t("profileTransfer.exportButton")}
      </LoadingButton>
    </div>
  );
}

/**
 * Read a Donut archive back as a new profile.
 *
 * The archive is inspected before anything is created, so the user sees what
 * they are about to add: its name, its browser version, and whether it carries
 * the browser data or only the configuration.
 */
export function ImportProfileArchive({
  onImported,
}: {
  onImported?: (profile: BrowserProfile) => void;
}) {
  const { t } = useTranslation();
  const [archive, setArchive] = useState<string | null>(null);
  const [manifest, setManifest] = useState<PortableManifest | null>(null);
  const [isBusy, setIsBusy] = useState(false);

  const handleChoose = async () => {
    const selected = await open({
      multiple: false,
      title: t("profileTransfer.importTitle"),
      filters: [
        {
          name: t("profileTransfer.archiveFilter"),
          extensions: [PROFILE_ARCHIVE_EXTENSION],
        },
      ],
    });
    if (typeof selected !== "string") return;

    setIsBusy(true);
    try {
      const preview = await invoke<{ manifest: PortableManifest }>(
        "preview_profile_archive",
        { path: selected },
      );
      setArchive(selected);
      setManifest(preview.manifest);
    } catch (error) {
      setArchive(null);
      setManifest(null);
      showToast({ type: "error", title: translateBackendError(t, error) });
    } finally {
      setIsBusy(false);
    }
  };

  const handleImport = async () => {
    if (!archive) return;
    setIsBusy(true);
    try {
      const profile = await invoke<BrowserProfile>("import_profile_archive", {
        path: archive,
      });
      showToast({
        type: "success",
        title: t("profileTransfer.imported", { name: profile.name }),
      });
      setArchive(null);
      setManifest(null);
      onImported?.(profile);
    } catch (error) {
      showToast({ type: "error", title: translateBackendError(t, error) });
    } finally {
      setIsBusy(false);
    }
  };

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        {t("profileTransfer.importDescription")}
      </p>
      <LoadingButton
        isLoading={isBusy && !manifest}
        variant="outline"
        onClick={() => {
          void handleChoose();
        }}
      >
        {t("profileTransfer.chooseArchive")}
      </LoadingButton>

      {manifest && (
        <div className="space-y-3 rounded-lg border p-4">
          <div className="space-y-1 text-sm">
            <p className="font-medium">{manifest.profile_name}</p>
            <p className="text-muted-foreground">
              {t("profileTransfer.archiveVersion", {
                browser: manifest.browser,
                version: manifest.version,
              })}
            </p>
            <p className="text-muted-foreground">
              {manifest.includes_data
                ? t("profileTransfer.archiveWithData")
                : t("profileTransfer.archiveConfigOnly")}
            </p>
          </div>
          <LoadingButton
            isLoading={isBusy}
            onClick={() => {
              void handleImport();
            }}
          >
            {t("profileTransfer.importButton")}
          </LoadingButton>
        </div>
      )}
    </div>
  );
}
