import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import {
  showDownloadToast,
  showFetchingToast,
  showSuccessToast,
  showErrorToast,
  dismissToast,
} from "../components/custom-toast";
import { getBrowserDisplayName } from "@/lib/browser-utils";

interface GithubRelease {
  tag_name: string;
  assets: Array<{
    name: string;
    browser_download_url: string;
    hash?: string;
  }>;
  published_at: string;
  is_alpha: boolean;
}

interface BrowserVersionInfo {
  version: string;
  is_prerelease: boolean;
  date: string;
}

interface DownloadProgress {
  browser: string;
  version: string;
  downloaded_bytes: number;
  total_bytes?: number;
  percentage: number;
  speed_bytes_per_sec: number;
  eta_seconds?: number;
  stage: string;
}

interface BrowserVersionsResult {
  versions: string[];
  new_versions_count?: number;
  total_versions_count: number;
}

interface VersionUpdateProgress {
  current_browser: string;
  total_browsers: number;
  completed_browsers: number;
  new_versions_found: number;
  browser_new_versions: number;
  status: string;
}

const isAlphaVersion = (version: string): boolean => {
  // Check for common alpha/beta/dev indicators
  const lowerVersion = version.toLowerCase();
  return (
    lowerVersion.includes("a") ||
    lowerVersion.includes("b") ||
    lowerVersion.includes("alpha") ||
    lowerVersion.includes("beta") ||
    lowerVersion.includes("dev") ||
    lowerVersion.includes("rc") ||
    lowerVersion.includes("pre") ||
    // Check for patterns like "139.0b1" or "140.0a1"
    /\d+\.\d+[ab]\d+/.test(lowerVersion)
  );
};

export function useBrowserDownload() {
  const [availableVersions, setAvailableVersions] = useState<GithubRelease[]>(
    []
  );
  const [downloadedVersions, setDownloadedVersions] = useState<string[]>([]);
  const [isDownloading, setIsDownloading] = useState(false);
  const [downloadProgress, setDownloadProgress] =
    useState<DownloadProgress | null>(null);
  const [isUpdatingVersions, setIsUpdatingVersions] = useState(false);

  // Listen for download progress events
  useEffect(() => {
    const unlisten = listen<DownloadProgress>("download-progress", (event) => {
      const progress = event.payload;
      setDownloadProgress(progress);

      const browserName = getBrowserDisplayName(progress.browser);

      // Check if this is an auto-update download to suppress completion toast
      const checkAutoUpdate = async () => {
        let isAutoUpdate = false;
        try {
          isAutoUpdate = await invoke<boolean>("is_auto_update_download", {
            browser: progress.browser,
            version: progress.version,
          });
        } catch (error) {
          console.error("Failed to check auto-update status:", error);
        }

        // Show toast with progress
        if (progress.stage === "downloading") {
          const speedMBps = (
            progress.speed_bytes_per_sec /
            (1024 * 1024)
          ).toFixed(1);
          const etaText = progress.eta_seconds
            ? formatTime(progress.eta_seconds)
            : "calculating...";

          showDownloadToast(browserName, progress.version, "downloading", {
            percentage: progress.percentage,
            speed: speedMBps,
            eta: etaText,
          });
        } else if (progress.stage === "extracting") {
          showDownloadToast(browserName, progress.version, "extracting");
        } else if (progress.stage === "verifying") {
          showDownloadToast(browserName, progress.version, "verifying");
        } else if (progress.stage === "completed") {
          // Suppress completion toast for auto-updates
          showDownloadToast(
            browserName,
            progress.version,
            "completed",
            undefined,
            {
              suppressCompletionToast: isAutoUpdate,
            }
          );
          setDownloadProgress(null);
        }
      };

      void checkAutoUpdate();
    });

    return () => {
      void unlisten.then((fn) => {
        fn();
      });
    };
  }, []);

  // Listen for version update progress events
  useEffect(() => {
    const unlisten = listen<VersionUpdateProgress>(
      "version-update-progress",
      (event) => {
        const progress = event.payload;

        if (progress.status === "updating") {
          setIsUpdatingVersions(true);
          if (progress.current_browser) {
            const browserName = getBrowserDisplayName(progress.current_browser);
            showFetchingToast(browserName, {
              id: `version-update-${progress.current_browser}`,
              description: "Fetching latest release information...",
            });
          }
        } else if (progress.status === "completed") {
          setIsUpdatingVersions(false);
          if (progress.new_versions_found > 0) {
            showSuccessToast(
              `Found ${progress.new_versions_found} new browser versions!`,
              {
                duration: 3000,
              }
            );
          }
          // Dismiss any update toasts
          toast.dismiss();
        } else if (progress.status === "error") {
          setIsUpdatingVersions(false);
          showErrorToast("Failed to check for new versions", {
            duration: 4000,
          });
          toast.dismiss();
        }
      }
    );

    return () => {
      void unlisten.then((fn) => {
        fn();
      });
    };
  }, []);

  const formatTime = (seconds: number): string => {
    if (seconds < 60) {
      return `${Math.round(seconds)}s`;
    } else if (seconds < 3600) {
      const minutes = Math.floor(seconds / 60);
      const remainingSeconds = Math.round(seconds % 60);
      return `${minutes}m ${remainingSeconds}s`;
    } else {
      const hours = Math.floor(seconds / 3600);
      const minutes = Math.floor((seconds % 3600) / 60);
      return `${hours}h ${minutes}m`;
    }
  };

  const formatBytes = (bytes: number): string => {
    if (bytes === 0) return "0 B";
    const k = 1024;
    const sizes = ["B", "KB", "MB", "GB"];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${Number.parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${
      sizes[i]
    }`;
  };

  const loadVersions = useCallback(async (browserStr: string) => {
    const browserName = getBrowserDisplayName(browserStr);

    // Show fetching toast
    const toastId = showFetchingToast(browserName, {
      id: `fetch-${browserStr}`,
    });

    try {
      const versionInfos = await invoke<BrowserVersionInfo[]>(
        "fetch_browser_versions_cached_first",
        { browserStr }
      );

      // Convert BrowserVersionInfo to GithubRelease format for compatibility
      const githubReleases: GithubRelease[] = versionInfos.map(
        (versionInfo) => ({
          tag_name: versionInfo.version,
          assets: [],
          published_at: versionInfo.date,
          is_alpha: versionInfo.is_prerelease,
        })
      );

      setAvailableVersions(githubReleases);
      dismissToast(toastId);
      return githubReleases;
    } catch (error) {
      console.error("Failed to load versions:", error);
      dismissToast(toastId);
      showErrorToast(`Failed to fetch ${browserName} versions`, {
        description:
          error instanceof Error ? error.message : "Unknown error occurred",
        duration: 4000,
      });
      throw error;
    }
  }, []);

  const loadVersionsWithNewCount = useCallback(async (browserStr: string) => {
    const browserName = getBrowserDisplayName(browserStr);

    try {
      // Get versions with new count info and cached detailed info
      const result = await invoke<BrowserVersionsResult>(
        "fetch_browser_versions_with_count_cached_first",
        { browserStr }
      );

      // Get detailed version info for compatibility
      const versionInfos = await invoke<BrowserVersionInfo[]>(
        "fetch_browser_versions_cached_first",
        { browserStr }
      );

      // Convert BrowserVersionInfo to GithubRelease format for compatibility
      const githubReleases: GithubRelease[] = versionInfos.map(
        (versionInfo) => ({
          tag_name: versionInfo.version,
          assets: [],
          published_at: versionInfo.date,
          is_alpha: versionInfo.is_prerelease,
        })
      );

      setAvailableVersions(githubReleases);

      // Show notification about new versions if any were found
      if (result.new_versions_count && result.new_versions_count > 0) {
        showSuccessToast(
          `Found ${result.new_versions_count} new ${browserName} versions!`,
          {
            duration: 3000,
            description: `Total available: ${result.total_versions_count} versions`,
          }
        );
      }

      return githubReleases;
    } catch (error) {
      console.error("Failed to load versions:", error);
      showErrorToast(`Failed to fetch ${browserName} versions`, {
        description:
          error instanceof Error ? error.message : "Unknown error occurred",
        duration: 4000,
      });
      throw error;
    }
  }, []);

  const loadDownloadedVersions = useCallback(async (browserStr: string) => {
    try {
      const downloadedVersions = await invoke<string[]>(
        "get_downloaded_browser_versions",
        { browserStr }
      );
      setDownloadedVersions(downloadedVersions);
      return downloadedVersions;
    } catch (error) {
      console.error("Failed to load downloaded versions:", error);
      throw error;
    }
  }, []);

  const downloadBrowser = useCallback(
    async (
      browserStr: string,
      version: string,
      suppressNotifications: boolean = false
    ) => {
      const browserName = getBrowserDisplayName(browserStr);
      setIsDownloading(true);

      try {
        await invoke("download_browser", { browserStr, version });
        await loadDownloadedVersions(browserStr);
      } catch (error) {
        console.error("Failed to download browser:", error);

        if (!suppressNotifications) {
          // Dismiss any existing download toast and show error
          dismissToast(`download-${browserStr}-${version}`);
          showErrorToast(`Failed to download ${browserName} ${version}`, {
            description:
              error instanceof Error ? error.message : "Unknown error occurred",
          });
        }
        throw error;
      } finally {
        setIsDownloading(false);
      }
    },
    [loadDownloadedVersions]
  );

  const isVersionDownloaded = useCallback(
    (version: string) => {
      return downloadedVersions.includes(version);
    },
    [downloadedVersions]
  );

  return {
    availableVersions,
    downloadedVersions,
    isDownloading,
    downloadProgress,
    isUpdatingVersions,
    loadVersions,
    loadVersionsWithNewCount,
    loadDownloadedVersions,
    downloadBrowser,
    isVersionDownloaded,
    formatBytes,
    formatTime,
  };
}
