import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentOS, type OperatingSystem } from "@/lib/platform";

// Platform-specific imports
let macOSPermissions:
  | typeof import("tauri-plugin-macos-permissions-api")
  | null = null;

// Dynamically import macOS permissions only when needed
const loadMacOSPermissions = async () => {
  if (macOSPermissions) return macOSPermissions;

  try {
    macOSPermissions = await import("tauri-plugin-macos-permissions-api");
    return macOSPermissions;
  } catch (error) {
    console.warn("Failed to load macOS permissions API:", error);
    return null;
  }
};

export type PermissionType = "microphone" | "camera";

interface UsePermissionsReturn {
  requestPermission: (type: PermissionType) => Promise<boolean>;
  isMicrophoneAccessGranted: boolean;
  isCameraAccessGranted: boolean;
  isInitialized: boolean;
  currentOS: OperatingSystem | null;
  requiresSystemPermissions: boolean;
}

export function usePermissions(active = true): UsePermissionsReturn {
  const [isMicrophoneAccessGranted, setIsMicrophoneAccessGranted] =
    useState(false);
  const [isCameraAccessGranted, setIsCameraAccessGranted] = useState(false);
  const [currentOS, setCurrentOS] = useState<OperatingSystem | null>(null);
  const [isInitialized, setIsInitialized] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Check permissions status
  const checkPermissions = useCallback(async () => {
    if (!currentOS) return;

    if (currentOS !== "macos") {
      // Windows/Linux - assume permissions are granted
      setIsMicrophoneAccessGranted(true);
      setIsCameraAccessGranted(true);
      setIsInitialized(true);
      return;
    }

    // macOS - use the permissions API
    try {
      const permissions = await loadMacOSPermissions();
      if (permissions) {
        const [micGranted, camGranted] = await Promise.all([
          permissions.checkMicrophonePermission(),
          permissions.checkCameraPermission(),
        ]);

        setIsMicrophoneAccessGranted(micGranted);
        setIsCameraAccessGranted(camGranted);
      }
    } catch (error) {
      console.error("Failed to check permissions on macOS:", error);
    } finally {
      setIsInitialized(true);
    }
  }, [currentOS]);

  // Request permission
  const requestPermission = useCallback(
    async (type: PermissionType): Promise<boolean> => {
      // Non-macOS platforms do not require this permission gate.
      if ((currentOS ?? getCurrentOS()) !== "macos") return true;

      // macOS - use the permissions API
      try {
        const permissions = await loadMacOSPermissions();
        if (!permissions) return false;

        const readPermission = async () => {
          const granted =
            type === "microphone"
              ? await permissions.checkMicrophonePermission()
              : await permissions.checkCameraPermission();
          if (type === "microphone") {
            setIsMicrophoneAccessGranted(granted);
          } else {
            setIsCameraAccessGranted(granted);
          }
          return granted;
        };

        if (type === "microphone") {
          await permissions.requestMicrophonePermission();
        } else {
          await permissions.requestCameraPermission();
        }

        // Read once immediately. The hook's macOS poll keeps watching for
        // delayed TCC propagation without holding this request (and any next
        // system prompt) open for several extra seconds.
        return readPermission();
      } catch (error) {
        console.error(`Failed to request ${type} permission on macOS:`, error);
        return false;
      }
    },
    [currentOS],
  );

  // Initialize platform detection and start interval checking
  useEffect(() => {
    setCurrentOS(getCurrentOS());
  }, []);

  // Set up interval checking when platform is determined.
  // On non-macOS platforms, permissions are always granted — a single check
  // is enough and we skip the interval entirely to avoid burning CPU.
  useEffect(() => {
    if (!currentOS || !active) return;

    // Initial check
    void checkPermissions();

    // Only poll on macOS where permissions can change at runtime
    if (currentOS !== "macos") return;

    intervalRef.current = setInterval(() => {
      void checkPermissions();
    }, 5000);

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [active, currentOS, checkPermissions]);

  return {
    requestPermission,
    isMicrophoneAccessGranted,
    isCameraAccessGranted,
    isInitialized,
    currentOS,
    requiresSystemPermissions: currentOS === "macos",
  };
}
