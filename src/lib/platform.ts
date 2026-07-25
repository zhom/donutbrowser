export type OperatingSystem = "macos" | "windows" | "linux" | "unknown";

type NavigatorWithUserAgentData = Navigator & {
  userAgentData?: {
    platform?: string;
  };
};

export function getCurrentOS(): OperatingSystem {
  if (typeof navigator === "undefined") return "unknown";

  const extendedNavigator = navigator as NavigatorWithUserAgentData;
  const platform =
    extendedNavigator.userAgentData?.platform ?? navigator.platform ?? "";
  const signature = `${platform} ${navigator.userAgent}`;

  if (/Windows|Win32|Win64/i.test(signature)) return "windows";
  if (/Mac|iPhone|iPad|iPod/i.test(signature)) return "macos";
  if (/Linux|X11/i.test(signature)) return "linux";
  return "unknown";
}

export function isMacOS(): boolean {
  return getCurrentOS() === "macos";
}
