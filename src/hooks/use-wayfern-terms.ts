import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";

interface UseWayfernTermsReturn {
  termsAccepted: boolean | null;
  isLoading: boolean;
  checkTerms: () => Promise<void>;
}

export function useWayfernTerms(): UseWayfernTermsReturn {
  const [termsAccepted, setTermsAccepted] = useState<boolean | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  const checkTerms = useCallback(async () => {
    try {
      const accepted = await invoke<boolean>("check_wayfern_terms_accepted");
      setTermsAccepted(accepted);
    } catch (error) {
      console.error("Failed to check terms acceptance:", error);
      setTermsAccepted(false);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    checkTerms();
  }, [checkTerms]);

  return {
    termsAccepted,
    isLoading,
    checkTerms,
  };
}
