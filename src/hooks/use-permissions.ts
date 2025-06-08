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

export interface UsePermissionsReturn {
  requestPermission: (type: PermissionType) => Promise<void>;
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
    async (type: PermissionType): Promise<void> => {
      if (!currentPlatform || currentPlatform !== "macos") return;

      // macOS - use the permissions API
      try {
        const permissions = await loadMacOSPermissions();
        if (!permissions) return;

        if (type === "microphone") {
          await permissions.requestMicrophonePermission();

          // Poll for permission status change
          const pollMicPermission = async () => {
            const granted = await permissions.checkMicrophonePermission();
            setIsMicrophoneAccessGranted(granted);

            if (!granted) {
              setTimeout(() => {
                void pollMicPermission();
              }, 1000);
            }
          };

          await pollMicPermission();
        }

        if (type === "camera") {
          await permissions.requestCameraPermission();

          // Poll for permission status change
          const pollCamPermission = async () => {
            const granted = await permissions.checkCameraPermission();
            setIsCameraAccessGranted(granted);

            if (!granted) {
              setTimeout(() => {
                void pollCamPermission();
              }, 1000);
            }
          };

          await pollCamPermission();
        }
      } catch (error) {
        console.error(`Failed to request ${type} permission on macOS:`, error);
      }
    },
    [currentPlatform],
  );

  // Initialize platform detection and start interval checking
  useEffect(() => {
    const initializePlatform = async () => {
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

    initializePlatform().catch(console.error);
  }, []);

  // Set up interval checking when platform is determined
  useEffect(() => {
    if (!currentPlatform) return;

    // Initial check
    void checkPermissions();

    // Set up 500ms interval for checking permissions
    intervalRef.current = setInterval(() => {
      void checkPermissions();
    }, 500);

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
