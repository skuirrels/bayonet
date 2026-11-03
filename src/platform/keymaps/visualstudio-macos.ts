/**
 * "Visual Studio (macOS)" keymap: the Visual Studio shortcuts with Ctrl read as ⌘
 * on the macOS IDEA keymap, leaving out the keys macOS keeps for itself (⌘Q, ⌘H,
 * ⌘M, ⌘Tab, ⌘`, ⌘Space), which no app receives.
 */
import { intellijMacos } from "./intellij-macos";
import { overlay, visualStudioAdditions, visualStudioOverrides } from "./visualstudio-windows";

/** First strokes macOS handles before the app sees them. */
const SYSTEM = new Set(["Meta+Q", "Meta+H", "Meta+M", "Meta+Tab", "Meta+Shift+Tab", "Meta+`", "Meta+Space"]);

function withMeta(binding: string): string | null {
  const strokes = binding.split(" ").map((s) => {
    const parts = s.split("+");
    if (parts.includes("Meta")) return null;
    return parts.map((p) => (p === "Ctrl" ? "Meta" : p)).join("+");
  });
  if (strokes.includes(null) || SYSTEM.has(strokes[0]!)) return null;
  return strokes.join(" ");
}

const translate = (map: Record<string, string[]>) =>
  Object.fromEntries(
    Object.entries(map).map(([id, keys]) => [id, keys.map(withMeta).filter((k): k is string => k !== null)]),
  );

export const visualStudioMacos = overlay(
  intellijMacos,
  {
    ...translate(visualStudioOverrides),
    // ⌘Space is Spotlight; ⌃Space as in the macOS IDEA keymap.
    "code.completion": ["Ctrl+Space", "Meta+J"],
    // ⌘, opens Settings on macOS.
    "navigate.searchEverywhere": ["Meta+T", "Shift Shift"],
    // ⌘Q quits; IDEA's ⇧⌘A.
    "navigate.findAction": ["Meta+Shift+A"],
    // ⌘Tab switches apps; ⌃Tab as on Windows.
    "navigate.switcher": ["Ctrl+Tab"],
    "navigate.switcherBack": ["Ctrl+Shift+Tab"],
    // ⌘M minimises: the outlining chords keep Ctrl.
    "code.fold": ["Ctrl+M Ctrl+M"],
    "code.foldAll": ["Ctrl+M Ctrl+O"],
    "code.unfoldAll": ["Ctrl+M Ctrl+L"],
  },
  { ...translate(visualStudioAdditions), "toolwindow.terminal": ["Ctrl+`"] },
);
