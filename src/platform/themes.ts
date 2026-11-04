/** Stable built-in ids shared by settings, quick switch and editor schemes. */
export const BUILTIN_THEMES = [
  { id: "light", label: "Islands Light", kind: "light" },
  { id: "dark", label: "Islands Dark", kind: "dark" },
  { id: "dark-hc", label: "High Contrast", kind: "dark" },
  { id: "solarized-light", label: "Solarized Light", kind: "light" },
  { id: "solarized-dark", label: "Solarized Dark", kind: "dark" },
  { id: "one-light", label: "One Light", kind: "light" },
  { id: "one-dark", label: "One Dark", kind: "dark" },
] as const;

export type BuiltinThemeId = (typeof BUILTIN_THEMES)[number]["id"];
export const BUILTIN_THEME_IDS: readonly BuiltinThemeId[] = BUILTIN_THEMES.map((t) => t.id);
export function isBuiltinTheme(value: unknown): value is BuiltinThemeId {
  return typeof value === "string" && (BUILTIN_THEME_IDS as readonly string[]).includes(value);
}
export function themeKind(id: BuiltinThemeId): "light" | "dark" {
  return BUILTIN_THEMES.find((t) => t.id === id)!.kind;
}
