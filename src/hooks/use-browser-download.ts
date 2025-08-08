import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { getBrowserDisplayName } from "@/lib/browser-utils";
import {
  dismissToast,
  showDownloadToast,
  showErrorToast,
  showSuccessToast,
} from "@/lib/toast-utils";

interface GithubRelease {
  tag_name: string;
  assets: {
    name: string;
    browser_download_url: string;
    hash?: string;
  }[];
  published_at: string;
  is_nightly: boolean;
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

export function useBrowserDownload() {
  const [availableVersions, setAvailableVersions] = useState<GithubRelease[]>(
    [],
  );
  const [downloadedVersions, setDownloadedVersions] = useState<string[]>([]);
  const [downloadingBrowsers, setDownloadingBrowsers] = useState<Set<string>>(
    new Set(),
  );
  const [downloadProgress, setDownloadProgress] =
    useState<DownloadProgress | null>(null);

  const formatTime = useCallback((seconds: number): string => {
    if (seconds < 60) {
      return `${Math.round(seconds)}s`;
    }
    if (seconds < 3600) {
      const minutes = Math.floor(seconds / 60);
      const remainingSeconds = Math.round(seconds % 60);
      return `${minutes}m ${remainingSeconds}s`;
    }
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    return `${hours}h ${minutes}m`;
  }, []);

  const formatBytes = useCallback((bytes: number): string => {
    if (bytes === 0) return "0 B";
    const k = 1024;
    const sizes = ["B", "KB", "MB", "GB"];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${Number.parseFloat((bytes / k ** i).toFixed(1))} ${sizes[i]}`;
  }, []);

  const loadVersions = useCallback(async (browserStr: string) => {
    const browserName = getBrowserDisplayName(browserStr);

    // Use a simple loading state instead of toast for version fetching
    console.log(`Fetching ${browserName} versions...`);

    try {
      const versionInfos = await invoke<BrowserVersionInfo[]>(
        "fetch_browser_versions_cached_first",
        { browserStr },
      );

      // Convert BrowserVersionInfo to GithubRelease format for compatibility
      const githubReleases: GithubRelease[] = versionInfos.map(
        (versionInfo) => ({
          tag_name: versionInfo.version,
          assets: [],
          published_at: versionInfo.date,
          is_nightly: versionInfo.is_prerelease,
        }),
      );

      setAvailableVersions(githubReleases);
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

  const loadVersionsWithNewCount = useCallback(async (browserStr: string) => {
    const browserName = getBrowserDisplayName(browserStr);

    try {
      // Get versions with new count info and cached detailed info
      const result = await invoke<BrowserVersionsResult>(
        "fetch_browser_versions_with_count_cached_first",
        { browserStr },
      );

      // Get detailed version info for compatibility
      const versionInfos = await invoke<BrowserVersionInfo[]>(
        "fetch_browser_versions_cached_first",
        { browserStr },
      );

      // Convert BrowserVersionInfo to GithubRelease format for compatibility
      const githubReleases: GithubRelease[] = versionInfos.map(
        (versionInfo) => ({
          tag_name: versionInfo.version,
          assets: [],
          published_at: versionInfo.date,
          is_nightly: versionInfo.is_prerelease,
        }),
      );

      setAvailableVersions(githubReleases);

      // Show notification about new versions if any were found
      if (result.new_versions_count && result.new_versions_count > 0) {
        showSuccessToast(
          `Found ${result.new_versions_count} new ${browserName} versions!`,
          {
            duration: 3000,
            description: `Total available: ${result.total_versions_count} versions`,
          },
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
        { browserStr },
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
      suppressNotifications = false,
    ) => {
      const browserName = getBrowserDisplayName(browserStr);
      setDownloadingBrowsers((prev) => new Set(prev).add(browserStr));

      try {
        // Check browser compatibility before attempting download
        const isSupported = await invoke<boolean>(
          "is_browser_supported_on_platform",
          { browserStr },
        );
        if (!isSupported) {
          const supportedBrowsers = await invoke<string[]>(
            "get_supported_browsers",
          );
          throw new Error(
            `${browserName} is not supported on your platform. Supported browsers: ${supportedBrowsers
              .map(getBrowserDisplayName)
              .join(", ")}`,
          );
        }

        await invoke("download_browser", { browserStr, version });
        await loadDownloadedVersions(browserStr);
      } catch (error) {
        console.error("Failed to download browser:", error);

        if (!suppressNotifications) {
          // Dismiss any existing download toast and show error
          dismissToast(`download-${browserStr}-${version}`);

          let errorMessage = "Unknown error occurred";
          if (error instanceof Error) {
            errorMessage = error.message;
          } else if (typeof error === "string") {
            errorMessage = error;
          } else if (error && typeof error === "object" && "message" in error) {
            errorMessage = String(error.message);
          }

          showErrorToast(`Failed to download ${browserName} ${version}`, {
            description: errorMessage,
          });
        }
        throw error;
      } finally {
        setDownloadingBrowsers((prev) => {
          const next = new Set(prev);
          next.delete(browserStr);
          return next;
        });
      }
    },
    [loadDownloadedVersions],
  );

  const isVersionDownloaded = useCallback(
    (version: string) => {
      return downloadedVersions.includes(version);
    },
    [downloadedVersions],
  );

  // Check if a browser type is currently downloading
  const isBrowserDownloading = useCallback(
    (browserStr: string) => {
      return downloadingBrowsers.has(browserStr);
    },
    [downloadingBrowsers],
  );

  // Legacy isDownloading for backwards compatibility
  const isDownloading = downloadingBrowsers.size > 0;

  // Listen for download progress events
  useEffect(() => {
    let unlistenFn: (() => void) | null = null;

    const setupListener = async () => {
      try {
        unlistenFn = await listen<DownloadProgress>(
          "download-progress",
          (event) => {
            const progress = event.payload;
            setDownloadProgress(progress);

            const browserName = getBrowserDisplayName(progress.browser);

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
              showDownloadToast(browserName, progress.version, "completed");
              setDownloadProgress(null);
            }
          },
        );
      } catch (error) {
        console.error("Failed to setup download progress listener:", error);
      }
    };

    setupListener();

    return () => {
      if (unlistenFn) {
        try {
          unlistenFn();
        } catch (error) {
          console.error("Failed to cleanup download progress listener:", error);
        }
      }
    };
  }, [formatTime]);

  return {
    availableVersions,
    downloadedVersions,
    isDownloading,
    isBrowserDownloading,
    downloadingBrowsers,
    downloadProgress,
    loadVersions,
    loadVersionsWithNewCount,
    loadDownloadedVersions,
    downloadBrowser,
    isVersionDownloaded,
    formatBytes,
    formatTime,
  };
}
