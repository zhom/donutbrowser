export interface ThemeColors extends Record<string, string> {
  "--background": string;
  "--foreground": string;
  "--card": string;
  "--card-foreground": string;
  "--popover": string;
  "--popover-foreground": string;
  "--primary": string;
  "--primary-foreground": string;
  "--secondary": string;
  "--secondary-foreground": string;
  "--muted": string;
  "--muted-foreground": string;
  "--accent": string;
  "--accent-foreground": string;
  "--destructive": string;
  "--destructive-foreground": string;
  "--border": string;
}

export interface Theme {
  id: string;
  name: string;
  colors: ThemeColors;
}

export const THEMES: Theme[] = [
  {
    id: "tokyo-night",
    name: "Tokyo Night",
    colors: {
      "--background": "#1a1b26",
      "--foreground": "#c0caf5",
      "--card": "#24283b",
      "--card-foreground": "#c0caf5",
      "--popover": "#24283b",
      "--popover-foreground": "#c0caf5",
      "--primary": "#7aa2f7",
      "--primary-foreground": "#1a1b26",
      "--secondary": "#2ac3de",
      "--secondary-foreground": "#1a1b26",
      "--muted": "#3b4261",
      "--muted-foreground": "#a9b1d6",
      "--accent": "#bb9af7",
      "--accent-foreground": "#1a1b26",
      "--destructive": "#f7768e",
      "--destructive-foreground": "#1a1b26",
      "--border": "#3b4261",
    },
  },
  {
    id: "dracula",
    name: "Dracula",
    colors: {
      "--background": "#282a36",
      "--foreground": "#f8f8f2",
      "--card": "#44475a",
      "--card-foreground": "#f8f8f2",
      "--popover": "#44475a",
      "--popover-foreground": "#f8f8f2",
      "--primary": "#bd93f9",
      "--primary-foreground": "#282a36",
      "--secondary": "#8be9fd",
      "--secondary-foreground": "#282a36",
      "--muted": "#6272a4",
      "--muted-foreground": "#f8f8f2",
      "--accent": "#ff79c6",
      "--accent-foreground": "#282a36",
      "--destructive": "#ff5555",
      "--destructive-foreground": "#f8f8f2",
      "--border": "#6272a4",
    },
  },
  {
    id: "matchalk",
    name: "Matchalk",
    colors: {
      "--background": "#273136",
      "--foreground": "#D1DED3",
      "--card": "#1c2427",
      "--card-foreground": "#D1DED3",
      "--popover": "#323e45",
      "--popover-foreground": "#D1DED3",
      "--primary": "#7eb08a",
      "--primary-foreground": "#273136",
      "--secondary": "#d2b48c",
      "--secondary-foreground": "#273136",
      "--muted": "#323e45",
      "--muted-foreground": "#7ea4b0",
      "--accent": "#d2b48c",
      "--accent-foreground": "#273136",
      "--destructive": "#ff819f",
      "--destructive-foreground": "#273136",
      "--border": "#304e37",
    },
  },
  {
    id: "houston",
    name: "Houston",
    colors: {
      "--background": "#17191e",
      "--foreground": "#f7f7f8",
      "--card": "#21252e",
      "--card-foreground": "#f7f7f8",
      "--popover": "#21252e",
      "--popover-foreground": "#f7f7f8",
      "--primary": "#5755d9",
      "--primary-foreground": "#f7f7f8",
      "--secondary": "#f25f4c",
      "--secondary-foreground": "#f7f7f8",
      "--muted": "#2a2e39",
      "--muted-foreground": "#9ca3af",
      "--accent": "#0ea5e9",
      "--accent-foreground": "#f7f7f8",
      "--destructive": "#ef4444",
      "--destructive-foreground": "#f7f7f8",
      "--border": "#2a2e39",
    },
  },
  {
    id: "ayu-dark",
    name: "Ayu Dark",
    colors: {
      "--background": "#0a0e14",
      "--foreground": "#b3b1ad",
      "--card": "#11151c",
      "--card-foreground": "#b3b1ad",
      "--popover": "#11151c",
      "--popover-foreground": "#b3b1ad",
      "--primary": "#39bae6",
      "--primary-foreground": "#0a0e14",
      "--secondary": "#ffb454",
      "--secondary-foreground": "#0a0e14",
      "--muted": "#1f2430",
      "--muted-foreground": "#5c6773",
      "--accent": "#d2a6ff",
      "--accent-foreground": "#0a0e14",
      "--destructive": "#f07178",
      "--destructive-foreground": "#b3b1ad",
      "--border": "#1f2430",
    },
  },
  {
    id: "ayu-light",
    name: "Ayu Light",
    colors: {
      "--background": "#fafafa",
      "--foreground": "#5c6773",
      "--card": "#ffffff",
      "--card-foreground": "#5c6773",
      "--popover": "#ffffff",
      "--popover-foreground": "#5c6773",
      "--primary": "#399ee6",
      "--primary-foreground": "#fafafa",
      "--secondary": "#fa8d3e",
      "--secondary-foreground": "#fafafa",
      "--muted": "#f0f0f0",
      "--muted-foreground": "#828c99",
      "--accent": "#a37acc",
      "--accent-foreground": "#fafafa",
      "--destructive": "#f07178",
      "--destructive-foreground": "#fafafa",
      "--border": "#e7eaed",
    },
  },
];

export const THEME_VARIABLES: Array<{ key: keyof ThemeColors; label: string }> =
  [
    { key: "--background", label: "Background" },
    { key: "--foreground", label: "Foreground" },
    { key: "--card", label: "Card" },
    { key: "--card-foreground", label: "Card FG" },
    { key: "--popover", label: "Popover" },
    { key: "--popover-foreground", label: "Popover FG" },
    { key: "--primary", label: "Primary" },
    { key: "--primary-foreground", label: "Primary FG" },
    { key: "--secondary", label: "Secondary" },
    { key: "--secondary-foreground", label: "Secondary FG" },
    { key: "--muted", label: "Muted" },
    { key: "--muted-foreground", label: "Muted FG" },
    { key: "--accent", label: "Accent" },
    { key: "--accent-foreground", label: "Accent FG" },
    { key: "--destructive", label: "Destructive" },
    { key: "--destructive-foreground", label: "Destructive FG" },
    { key: "--border", label: "Border" },
  ];

export function getThemeById(id: string): Theme | undefined {
  return THEMES.find((theme) => theme.id === id);
}

export function getThemeByColors(
  colors: Record<string, string>,
): Theme | undefined {
  return THEMES.find((theme) => {
    return THEME_VARIABLES.every(({ key }) => {
      return theme.colors[key] === colors[key];
    });
  });
}

export function applyThemeColors(colors: Record<string, string>): void {
  const root = document.documentElement;
  Object.entries(colors).forEach(([key, value]) => {
    root.style.setProperty(key, value, "important");
  });
}

export function clearThemeColors(): void {
  const root = document.documentElement;
  THEME_VARIABLES.forEach(({ key }) => {
    root.style.removeProperty(key as string);
  });
}
