import { useCallback, useEffect, useRef, useState } from "react";

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
}

export function usePermissions(): UsePermissionsReturn {
  const [isMicrophoneAccessGranted, setIsMicrophoneAccessGranted] =
    useState(false);
  const [isCameraAccessGranted, setIsCameraAccessGranted] = useState(false);
  const [currentPlatform, setCurrentPlatform] = useState<string | null>(null);
  const [isInitialized, setIsInitialized] = useState(false);
  const intervalRef = useRef<NodeJS.Timeout | null>(null);

  // Check permissions status
  const checkPermissions = useCallback(async () => {
    if (!currentPlatform) return;

    if (currentPlatform !== "macos") {
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
        setIsInitialized(true);
      }
    } catch (error) {
      console.error("Failed to check permissions on macOS:", error);
      setIsInitialized(true);
    }
  }, [currentPlatform]);

  // Request permission
  const requestPermission = useCallback(
    async (type: PermissionType): Promise<boolean> => {
      // Non-macOS platforms do not require this permission gate.
      if (!currentPlatform || currentPlatform !== "macos") return true;

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

        for (let attempt = 0; attempt < 8; attempt += 1) {
          const granted = await readPermission();
          if (granted) return true;
          await new Promise((resolve) => setTimeout(resolve, 1000));
        }

        return readPermission();
      } catch (error) {
        console.error(`Failed to request ${type} permission on macOS:`, error);
        return false;
      }
    },
    [currentPlatform],
  );

  // Initialize platform detection and start interval checking
  useEffect(() => {
    const initializePlatform = () => {
      try {
        // Detect platform - on macOS we need permissions, on others we don't
        const userAgent = navigator.userAgent;
        let platformName = "unknown";

        if (userAgent.includes("Mac")) {
          platformName = "macos";
        } else if (userAgent.includes("Win")) {
          platformName = "windows";
        } else if (userAgent.includes("Linux")) {
          platformName = "linux";
        }

        setCurrentPlatform(platformName);
      } catch (error) {
        console.error("Failed to detect platform:", error);
        // Fallback - assume non-macOS
        setCurrentPlatform("unknown");
      }
    };

    initializePlatform();
  }, []);

  // Set up interval checking when platform is determined.
  // On non-macOS platforms, permissions are always granted — a single check
  // is enough and we skip the interval entirely to avoid burning CPU.
  useEffect(() => {
    if (!currentPlatform) return;

    // Initial check
    void checkPermissions();

    // Only poll on macOS where permissions can change at runtime
    if (currentPlatform !== "macos") return;

    intervalRef.current = setInterval(() => {
      void checkPermissions();
    }, 5000);

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [currentPlatform, checkPermissions]);

  return {
    requestPermission,
    isMicrophoneAccessGranted,
    isCameraAccessGranted,
    isInitialized,
  };
}
