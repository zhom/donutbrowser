import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
      const [accepted, downloaded] = await Promise.all([
        invoke<boolean>("check_wayfern_terms_accepted"),
        invoke<boolean>("check_wayfern_downloaded"),
      ]);
      // Only require terms when Wayfern is downloaded and terms not accepted
      if (!downloaded) {
        setTermsAccepted(true);
      } else {
        setTermsAccepted(accepted);
      }
    } catch (error) {
      console.error("Failed to check terms acceptance:", error);
      setTermsAccepted(false);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void checkTerms();
  }, [checkTerms]);

  // The backend announces every acceptance, including ones the dialog did
  // not drive (the REST API, an automation session), so the gate lifts
  // without a restart.
  useEffect(() => {
    let active = true;
    const subscription = listen("wayfern-terms-accepted", () => {
      if (active) void checkTerms();
    });
    return () => {
      active = false;
      void subscription.then((unlisten) => unlisten());
    };
  }, [checkTerms]);

  return {
    termsAccepted,
    isLoading,
    checkTerms,
  };
}
