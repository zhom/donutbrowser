"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";

interface DownloadProgress {
  browser: string;
  version: string;
  downloaded_bytes: number;
  total_bytes: number | null;
  percentage: number;
  speed_bytes_per_sec: number;
  eta_seconds?: number | null;
  stage: string;
}

export type SetupPhase = "downloading" | "extracting" | "ready" | "error";

export type SetupErrorStage =
  | "downloading"
  | "extracting"
  | "verifying"
  | "other";

export interface SetupError {
  stage: SetupErrorStage;
}

// The backend emits a real percentage only while downloading; extraction sends
// a single "extracting" event with no incremental progress (it takes ~2 min).
// So we estimate extraction progress from elapsed time vs. a learned average,
// seeded at 2 minutes and refined with the real durations we record.
const DEFAULT_EXTRACT_MS = 2 * 60 * 1000;
const MAX_SAMPLES = 5; // the 2-min seed + up to 4 most recent real durations

const storageKey = (browser: string) => `donut.extractDurations.${browser}`;

function readDurations(browser: string): number[] {
  try {
    const raw = localStorage.getItem(storageKey(browser));
    const arr = raw ? (JSON.parse(raw) as unknown) : null;
    if (
      Array.isArray(arr) &&
      arr.length > 0 &&
      arr.every((n) => typeof n === "number" && n > 0)
    ) {
      return arr as number[];
    }
  } catch {
    // fall through to the seed
  }
  return [DEFAULT_EXTRACT_MS];
}

function recordDuration(browser: string, ms: number) {
  if (!(ms > 0)) return;
  const current = readDurations(browser);
  // Keep the 2-min seed as the first value, then the most recent real samples.
  const samples =
    current[0] === DEFAULT_EXTRACT_MS ? current.slice(1) : current;
  const next = [
    DEFAULT_EXTRACT_MS,
    ...[...samples, ms].slice(-(MAX_SAMPLES - 1)),
  ];
  try {
    localStorage.setItem(storageKey(browser), JSON.stringify(next));
  } catch {
    // ignore persistence failures
  }
}

function average(values: number[]): number {
  return values.reduce((a, b) => a + b, 0) / values.length;
}

// Map a backend stage to the error stage we report when something fails.
function toErrorStage(stage: string): SetupErrorStage {
  switch (stage) {
    case "downloading":
      return "downloading";
    case "extracting":
      return "extracting";
    case "verifying":
      return "verifying";
    default:
      return "other";
  }
}

/**
 * Tracks first-launch setup of a browser: real download progress plus an
 * estimated extraction progress (no countdown timer, percentages only).
 * `active` should be true while the owning dialog is open.
 */
export function useBrowserSetup(browser: string, active: boolean) {
  const [phase, setPhase] = useState<SetupPhase>("downloading");
  // Download metrics straight from the latest "downloading" event.
  const [downloadPercent, setDownloadPercent] = useState(0);
  const [downloadedBytes, setDownloadedBytes] = useState(0);
  const [totalBytes, setTotalBytes] = useState<number | null>(null);
  const [speedBytesPerSec, setSpeedBytesPerSec] = useState(0);
  const [etaSeconds, setEtaSeconds] = useState<number | null>(null);
  // Estimated extraction progress (percentages only, capped at 99 until done).
  const [extractionPercent, setExtractionPercent] = useState(0);
  const [extractionOvertime, setExtractionOvertime] = useState(false);
  const [error, setError] = useState<SetupError | null>(null);

  const extractStartRef = useRef<number | null>(null);
  const estimateRef = useRef(DEFAULT_EXTRACT_MS);
  // Fallback bookkeeping so a listener that mounts mid-flight (and therefore
  // misses the single "extracting" event) can still show extraction progress.
  const sawDownloadingRef = useRef(false);
  const lastProgressAtRef = useRef<number | null>(null);
  const lastDownloadPercentRef = useRef(0);
  // The last non-terminal stage we observed, used to label an error.
  const lastStageRef = useRef<string>("downloading");
  // Set once a terminal state (ready/error) is reached. Stops the tick so the
  // mid-flight extraction fallback can't re-arm and fight the readiness poll
  // (which would oscillate "ready" ↔ "Almost finished" forever).
  const doneRef = useRef(false);

  useEffect(() => {
    if (!active) {
      // Fully reset when the owning dialog closes.
      setPhase("downloading");
      setDownloadPercent(0);
      setDownloadedBytes(0);
      setTotalBytes(null);
      setSpeedBytesPerSec(0);
      setEtaSeconds(null);
      setExtractionPercent(0);
      setExtractionOvertime(false);
      setError(null);
      extractStartRef.current = null;
      sawDownloadingRef.current = false;
      lastProgressAtRef.current = null;
      lastDownloadPercentRef.current = 0;
      lastStageRef.current = "downloading";
      doneRef.current = false;
      return;
    }
    let alive = true;
    estimateRef.current = average(readDurations(browser));
    extractStartRef.current = null;
    sawDownloadingRef.current = false;
    lastProgressAtRef.current = null;
    lastDownloadPercentRef.current = 0;
    lastStageRef.current = "downloading";
    doneRef.current = false;

    const finishExtraction = () => {
      if (extractStartRef.current != null) {
        recordDuration(browser, Date.now() - extractStartRef.current);
        extractStartRef.current = null;
      }
    };

    const unlistenPromise = listen<DownloadProgress>(
      "download-progress",
      (event) => {
        if (!alive) return;
        const p = event.payload;
        if (p.browser !== browser) return;
        switch (p.stage) {
          case "downloading":
            lastStageRef.current = "downloading";
            sawDownloadingRef.current = true;
            lastProgressAtRef.current = Date.now();
            lastDownloadPercentRef.current = p.percentage;
            setPhase("downloading");
            setDownloadPercent(Math.round(p.percentage));
            setDownloadedBytes(p.downloaded_bytes);
            setTotalBytes(p.total_bytes ?? null);
            setSpeedBytesPerSec(p.speed_bytes_per_sec);
            setEtaSeconds(p.eta_seconds ?? null);
            break;
          case "extracting":
            lastStageRef.current = "extracting";
            if (extractStartRef.current == null) {
              extractStartRef.current = Date.now();
            }
            lastProgressAtRef.current = Date.now();
            setPhase("extracting");
            break;
          case "verifying":
            lastStageRef.current = "verifying";
            finishExtraction();
            // Verification is the tail of extraction; keep the bar near full
            // but don't claim "ready" until "completed" arrives.
            setPhase("extracting");
            setExtractionPercent(99);
            break;
          case "completed":
            doneRef.current = true;
            finishExtraction();
            setPhase("ready");
            setExtractionPercent(100);
            setExtractionOvertime(false);
            setError(null);
            break;
          case "error":
            doneRef.current = true;
            finishExtraction();
            setPhase("error");
            setError({ stage: toErrorStage(lastStageRef.current) });
            break;
          case "cancelled":
            // Treat a cancellation like an error so the dialog can offer retry.
            doneRef.current = true;
            finishExtraction();
            setPhase("error");
            setError({ stage: "other" });
            break;
          default:
            break;
        }
      },
    );

    // Authoritative completion signal: poll the registry. The "completed" event
    // is only a fast-path — we never rely on it alone. This MUST be a recurring
    // interval rather than a one-shot loop: independent firings mean a single
    // invoke that stalls during heavy extraction can't kill detection, it keeps
    // confirming readiness so retry() re-detects an already-downloaded browser
    // without restarting the effect, and it covers a browser downloaded before
    // this hook mounted. setPhase("ready") is idempotent, so re-confirming is
    // free (React bails out when state is unchanged).
    let checkingReady = false;
    const checkReady = async () => {
      if (!alive || checkingReady) return;
      checkingReady = true;
      try {
        const versions = await invoke<string[]>(
          "get_downloaded_browser_versions",
          { browserStr: browser },
        );
        if (alive && versions.length > 0) {
          doneRef.current = true;
          finishExtraction();
          setPhase("ready");
          setExtractionPercent(100);
          setExtractionOvertime(false);
          setError(null);
        }
      } catch (err) {
        console.error("Failed to check browser download status:", err);
      } finally {
        checkingReady = false;
      }
    };
    void checkReady();
    const readyPoll = setInterval(() => {
      void checkReady();
    }, 1000);

    // Drive the estimated extraction percentage while extracting.
    const tick = setInterval(() => {
      if (!alive || doneRef.current) return;
      // If the download visibly finished but we never saw the (single)
      // "extracting" event, start estimating extraction anyway — anchored to
      // the last download event, which is roughly when extraction began.
      if (
        extractStartRef.current == null &&
        sawDownloadingRef.current &&
        lastDownloadPercentRef.current >= 99 &&
        lastProgressAtRef.current != null &&
        Date.now() - lastProgressAtRef.current > 1200
      ) {
        extractStartRef.current = lastProgressAtRef.current;
        lastStageRef.current = "extracting";
        setPhase("extracting");
      }
      if (extractStartRef.current == null) return;
      const elapsed = Date.now() - extractStartRef.current;
      const est = estimateRef.current || DEFAULT_EXTRACT_MS;
      if (elapsed >= est) {
        // We've blown past the estimate — hold at 99 and flag overtime so the
        // dialog can show "Almost finished" instead of a stalled number.
        setExtractionPercent(99);
        setExtractionOvertime(true);
      } else {
        setExtractionPercent(Math.min(99, Math.round((elapsed / est) * 100)));
        setExtractionOvertime(false);
      }
    }, 250);

    return () => {
      alive = false;
      clearInterval(tick);
      clearInterval(readyPoll);
      void unlistenPromise.then((u) => {
        u();
      });
    };
  }, [browser, active]);

  const retry = useCallback(() => {
    // Reset visible state and the bookkeeping refs, then kick off the download
    // again. The effect's event listener and registry poll stay alive the whole
    // time the dialog is open, so they pick up the fresh attempt — no need to
    // restart the effect.
    setPhase("downloading");
    setDownloadPercent(0);
    setDownloadedBytes(0);
    setTotalBytes(null);
    setSpeedBytesPerSec(0);
    setEtaSeconds(null);
    setExtractionPercent(0);
    setExtractionOvertime(false);
    setError(null);
    extractStartRef.current = null;
    sawDownloadingRef.current = false;
    lastProgressAtRef.current = null;
    lastDownloadPercentRef.current = 0;
    lastStageRef.current = "downloading";
    doneRef.current = false;
    void (async () => {
      try {
        await invoke("ensure_active_browsers_downloaded");
      } catch (err) {
        console.error("Failed to re-trigger browser setup:", err);
        setPhase("error");
        setError({ stage: "other" });
      }
    })();
  }, []);

  return {
    phase,
    downloadPercent,
    downloadedBytes,
    totalBytes,
    speedBytesPerSec,
    etaSeconds,
    extractionPercent,
    extractionOvertime,
    ready: phase === "ready",
    error,
    retry,
  };
}
