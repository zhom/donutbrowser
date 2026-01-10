import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";

export interface TrialStatusActive {
  type: "Active";
  remaining_seconds: number;
  days_remaining: number;
  hours_remaining: number;
  minutes_remaining: number;
}

export interface TrialStatusExpired {
  type: "Expired";
}

export type TrialStatus = TrialStatusActive | TrialStatusExpired;

interface UseCommercialTrialReturn {
  trialStatus: TrialStatus | null;
  hasAcknowledged: boolean;
  isLoading: boolean;
  checkTrialStatus: () => Promise<void>;
}

export function useCommercialTrial(): UseCommercialTrialReturn {
  const [trialStatus, setTrialStatus] = useState<TrialStatus | null>(null);
  const [hasAcknowledged, setHasAcknowledged] = useState(true);
  const [isLoading, setIsLoading] = useState(true);

  const checkTrialStatus = useCallback(async () => {
    try {
      const [status, acknowledged] = await Promise.all([
        invoke<TrialStatus>("get_commercial_trial_status"),
        invoke<boolean>("has_acknowledged_trial_expiration"),
      ]);
      setTrialStatus(status);
      setHasAcknowledged(acknowledged);
    } catch (error) {
      console.error("Failed to check trial status:", error);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    checkTrialStatus();

    // Check trial status every minute to update the countdown
    const interval = setInterval(checkTrialStatus, 60000);
    return () => clearInterval(interval);
  }, [checkTrialStatus]);

  return {
    trialStatus,
    hasAcknowledged,
    isLoading,
    checkTrialStatus,
  };
}
