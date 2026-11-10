export type ThemeMode = "dark" | "light";
export type ThemeSurface = "solid" | "glass";
export type ButtonStyle = "solid" | "soft" | "outline" | "glass";
export type ThemePresetId = "graphite" | "aurora" | "glacier" | "ember";

export interface ThemeSettings {
  version: 1;
  preset: ThemePresetId | "custom";
  seed: string;
  mode: ThemeMode;
  surface: ThemeSurface;
  buttonStyle: ButtonStyle;
  opacity: number;
  blur: number;
}

export interface ThemePreset {
  id: ThemePresetId;
  name: string;
  description: string;
  settings: Readonly<ThemeSettings>;
}

export const themeStorageKey = "enogram.appearance.v1";
export const themeLimits = Object.freeze({ minOpacity: 68, maxOpacity: 100, minBlur: 0, maxBlur: 28 });

export const themePresets: readonly ThemePreset[] = [
  {
    id: "graphite",
    name: "Graphite",
    description: "Sakin, keskin, koyu",
    settings: { version: 1, preset: "graphite", seed: "#75AEF2", mode: "dark", surface: "solid", buttonStyle: "soft", opacity: 94, blur: 16 },
  },
  {
    id: "aurora",
    name: "Aurora",
    description: "Mor ışık, sıvı cam",
    settings: { version: 1, preset: "aurora", seed: "#B39AF8", mode: "dark", surface: "glass", buttonStyle: "glass", opacity: 78, blur: 24 },
  },
  {
    id: "glacier",
    name: "Glacier",
    description: "Aydınlık, serin cam",
    settings: { version: 1, preset: "glacier", seed: "#24789B", mode: "light", surface: "glass", buttonStyle: "soft", opacity: 86, blur: 22 },
  },
  {
    id: "ember",
    name: "Ember",
    description: "Sıcak bakır, koyu cam",
    settings: { version: 1, preset: "ember", seed: "#EBA575", mode: "dark", surface: "glass", buttonStyle: "outline", opacity: 90, blur: 16 },
  },
];

export const defaultThemeSettings: Readonly<ThemeSettings> = Object.freeze({ ...themePresets[0].settings });

/** Only normalized hex colors enter CSS or persistence. */
export function normalizeHexColor(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const hex = value.trim().replace(/^#/, "");
  if (/^[\da-f]{3}$/i.test(hex)) return `#${[...hex].map((digit) => digit + digit).join("").toUpperCase()}`;
  return /^[\da-f]{6}$/i.test(hex) ? `#${hex.toUpperCase()}` : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function boundedNumber(value: unknown, min: number, max: number, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? Math.min(max, Math.max(min, Math.round(value))) : fallback;
}

export function settingsForPreset(id: ThemePresetId): ThemeSettings {
  return { ...(themePresets.find((preset) => preset.id === id) ?? themePresets[0]).settings };
}

export function normalizeThemeSettings(value: unknown): ThemeSettings {
  if (!isRecord(value) || value.version !== 1) return { ...defaultThemeSettings };
  const preset = themePresets.find((entry) => entry.id === value.preset);
  const base = preset?.settings ?? defaultThemeSettings;
  const seed = normalizeHexColor(value.seed) ?? base.seed;
  const mode = value.mode === "light" || value.mode === "dark" ? value.mode : base.mode;
  const surface = value.surface === "solid" || value.surface === "glass" ? value.surface : base.surface;
  const buttonStyle = value.buttonStyle === "solid" || value.buttonStyle === "soft" || value.buttonStyle === "outline" || value.buttonStyle === "glass"
    ? value.buttonStyle : base.buttonStyle;
  return {
    version: 1,
    preset: preset && preset.settings.seed === seed && preset.settings.mode === mode ? preset.id : "custom",
    seed,
    mode,
    surface,
    buttonStyle,
    opacity: boundedNumber(value.opacity, themeLimits.minOpacity, themeLimits.maxOpacity, base.opacity),
    blur: boundedNumber(value.blur, themeLimits.minBlur, themeLimits.maxBlur, base.blur),
  };
}

type Rgb = [number, number, number];

function rgbFromHex(hex: string): Rgb {
  const normalized = normalizeHexColor(hex);
  if (!normalized) throw new TypeError("Expected a valid hex color.");
  return [1, 3, 5].map((index) => Number.parseInt(normalized.slice(index, index + 2), 16)) as Rgb;
}

function hexFromRgb(rgb: Rgb): string {
  return `#${rgb.map((channel) => Math.round(Math.min(255, Math.max(0, channel))).toString(16).padStart(2, "0")).join("").toUpperCase()}`;
}

function mix(first: string, second: string, amount: number): string {
  const from = rgbFromHex(first);
  const to = rgbFromHex(second);
  return hexFromRgb(from.map((channel, index) => channel + (to[index] - channel) * amount) as Rgb);
}

export function relativeLuminance(hex: string): number {
  const linear = rgbFromHex(hex).map((channel) => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
}

export function contrastRatio(foreground: string, background: string): number {
  const first = relativeLuminance(foreground);
  const second = relativeLuminance(background);
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

function hueOf(hex: string): number {
  const [red, green, blue] = rgbFromHex(hex).map((value) => value / 255);
  const high = Math.max(red, green, blue);
  const low = Math.min(red, green, blue);
  const delta = high - low;
  if (delta < 0.025) return 215;
  const sector = high === red ? (green - blue) / delta : high === green ? (blue - red) / delta + 2 : (red - green) / delta + 4;
  return (sector * 60 + 360) % 360;
}

function hsl(hue: number, saturation: number, lightness: number): string {
  const chroma = (1 - Math.abs(2 * lightness - 1)) * saturation;
  const section = hue / 60;
  const secondary = chroma * (1 - Math.abs(section % 2 - 1));
  const offset = lightness - chroma / 2;
  const segments: Rgb[] = [[chroma, secondary, 0], [secondary, chroma, 0], [0, chroma, secondary], [0, secondary, chroma], [secondary, 0, chroma], [chroma, 0, secondary]];
  return hexFromRgb(segments[Math.floor(section) % 6].map((channel) => (channel + offset) * 255) as Rgb);
}

function readableColor(color: string, backgrounds: readonly string[], mode: ThemeMode, minimum = 4.5): string {
  const endpoint = mode === "dark" ? "#FFFFFF" : "#000000";
  for (let step = 0; step <= 100; step += 1) {
    const candidate = mix(color, endpoint, step / 100);
    if (backgrounds.every((background) => contrastRatio(candidate, background) >= minimum)) return candidate;
  }
  return endpoint;
}

/** Adjusts hue brightness, including its tinted background, before exposing a semantic color. */
function semanticColor(seed: string, panel: string, backgrounds: readonly string[], mode: ThemeMode): { foreground: string; background: string } {
  const endpoint = mode === "dark" ? "#FFFFFF" : "#000000";
  const weight = mode === "dark" ? 0.13 : 0.08;
  for (let step = 0; step <= 100; step += 1) {
    const foreground = mix(seed, endpoint, step / 100);
    const background = mix(panel, foreground, weight);
    if ([...backgrounds, background].every((surface) => contrastRatio(foreground, surface) >= 4.65)) return { foreground, background };
  }
  return { foreground: endpoint, background: mix(panel, endpoint, weight) };
}

export type ThemeTokens = Record<`--${string}`, string>;

/** Opaque semantic colors stay legible; CSS adds glass only to the app's outer surfaces. */
export function createThemeTokens(value: ThemeSettings): ThemeTokens {
  const settings = normalizeThemeSettings(value);
  const { mode } = settings;
  const dark = mode === "dark";
  const hue = hueOf(settings.seed);
  const canvas = hsl(hue, dark ? 0.2 : 0.21, dark ? 0.058 : 0.94);
  const panel = hsl(hue, dark ? 0.18 : 0.18, dark ? 0.087 : 0.978);
  const raised = hsl(hue, dark ? 0.17 : 0.15, dark ? 0.126 : 1);
  const low = hsl(hue, dark ? 0.2 : 0.17, dark ? 0.069 : 0.956);
  const baseSurfaces = [canvas, panel, raised, low];
  const accent = semanticColor(settings.seed, panel, baseSurfaces, mode);
  const selection = mix(panel, accent.foreground, dark ? 0.18 : 0.12);
  const textSurfaces = [...baseSurfaces, selection, accent.background];
  const text = readableColor(dark ? "#E8EDF5" : "#202A36", textSurfaces, mode, 7);
  const muted = readableColor(dark ? "#9AA8BB" : "#5C6978", textSurfaces, mode, 4.65);
  const dim = readableColor(dark ? "#8B9AAE" : "#667483", textSurfaces, mode, 4.5);
  const success = semanticColor(dark ? "#79D8A5" : "#237747", low, baseSurfaces, mode);
  const danger = semanticColor(dark ? "#F58D99" : "#B83D4D", low, baseSurfaces, mode);
  const warning = semanticColor(dark ? "#E7BB68" : "#8B5F13", low, baseSurfaces, mode);
  const whiteContrast = contrastRatio("#FFFFFF", accent.foreground);
  const blackContrast = contrastRatio("#070B10", accent.foreground);
  const accentContrast = whiteContrast >= blackContrast ? "#FFFFFF" : "#070B10";
  const graphColors = [accent.foreground, "#B896FF", "#65D4B3", "#EBC379", "#E8A5C6", "#80D7E3", "#ED9B83", "#B3BCCB"];
  const graphTokens = Object.fromEntries(graphColors.map((color, index) => [
    `--graph-${index + 1}`,
    readableColor(color, textSurfaces, mode, 3.15),
  ]));

  return {
    "--canvas": canvas,
    "--panel": panel,
    "--panel-raised": raised,
    "--panel-low": low,
    "--line": mix(panel, text, dark ? 0.18 : 0.2),
    "--line-soft": mix(panel, text, dark ? 0.095 : 0.11),
    "--text": text,
    "--muted": muted,
    "--dim": dim,
    "--blue": accent.foreground,
    "--blue-soft": accent.background,
    "--accent-contrast": accentContrast,
    "--selection": selection,
    "--green": success.foreground,
    "--green-soft": success.background,
    "--red": danger.foreground,
    "--red-soft": danger.background,
    "--amber": warning.foreground,
    "--amber-soft": warning.background,
    "--glass-opacity": String(settings.opacity / 100),
    "--glass-blur": `${settings.blur}px`,
    "--glass-highlight": dark ? "rgb(255 255 255 / 0.11)" : "rgb(255 255 255 / 0.9)",
    "--shadow-color": dark ? "rgb(0 0 0 / 0.38)" : "rgb(27 43 62 / 0.14)",
    "--panel-rgb": rgbFromHex(panel).join(" "),
    "--canvas-rgb": rgbFromHex(canvas).join(" "),
    ...graphTokens,
  };
}

export interface ThemeStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export type ThemeStorageStatus = "available" | "recovered" | "unavailable";
export interface LoadedTheme {
  settings: ThemeSettings;
  storageStatus: ThemeStorageStatus;
}

function resolveStorage(storage?: ThemeStorage | null): ThemeStorage | null {
  if (storage !== undefined) return storage;
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}

export function loadThemeSettings(storage?: ThemeStorage | null): LoadedTheme {
  const target = resolveStorage(storage);
  if (!target) return { settings: { ...defaultThemeSettings }, storageStatus: "unavailable" };
  try {
    const raw = target.getItem(themeStorageKey);
    if (raw === null) return { settings: { ...defaultThemeSettings }, storageStatus: "available" };
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      return { settings: { ...defaultThemeSettings }, storageStatus: "recovered" };
    }
    const settings = normalizeThemeSettings(parsed);
    const valid = isRecord(parsed) && Object.entries(settings).every(([key, value]) => parsed[key] === value);
    return { settings, storageStatus: valid ? "available" : "recovered" };
  } catch {
    return { settings: { ...defaultThemeSettings }, storageStatus: "unavailable" };
  }
}

export function saveThemeSettings(settings: ThemeSettings, storage?: ThemeStorage | null): boolean {
  const target = resolveStorage(storage);
  if (!target) return false;
  try {
    target.setItem(themeStorageKey, JSON.stringify(normalizeThemeSettings(settings)));
    return true;
  } catch {
    return false;
  }
}

export interface ThemeTarget {
  style: { setProperty(name: string, value: string): void };
  setAttribute(name: string, value: string): void;
}

export function applyTheme(settings: ThemeSettings, target?: ThemeTarget): void {
  const root = target ?? (typeof document === "undefined" ? null : document.documentElement);
  if (!root) return;
  const validated = normalizeThemeSettings(settings);
  for (const [name, value] of Object.entries(createThemeTokens(validated))) root.style.setProperty(name, value);
  root.style.setProperty("color-scheme", validated.mode);
  root.setAttribute("data-theme", validated.mode);
  root.setAttribute("data-surface", validated.surface);
  root.setAttribute("data-button-style", validated.buttonStyle);
}

/** Call once before React renders to avoid a flash when restoring a light theme. */
export function initializeTheme(target?: ThemeTarget, storage?: ThemeStorage | null): LoadedTheme {
  const loaded = loadThemeSettings(storage);
  applyTheme(loaded.settings, target);
  return loaded;
}
