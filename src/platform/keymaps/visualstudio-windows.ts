/**
 * "Visual Studio" keymap: the Windows IDEA keymap with Visual Studio's default
 * shortcuts (Microsoft's published "Default keyboard shortcuts in Visual Studio")
 * for the actions both have. A key Visual Studio gives one action is taken from
 * every other action of the parent, so nothing is bound twice by accident.
 * Where Visual Studio reuses a key by state (F5 starts debugging, or continues
 * while paused), both actions keep it: the one enabled now runs.
 */
import { intellijWindows } from "./intellij-windows";

export const visualStudioOverrides: Record<string, string[]> = {
  // Navigation
  "navigate.searchEverywhere": ["Ctrl+T", "Ctrl+,", "Shift Shift"],
  "navigate.gotoFile": ["Ctrl+Shift+T", "Ctrl+1 Ctrl+F"],
  "navigate.gotoClass": ["Ctrl+1 Ctrl+T"],
  "navigate.gotoSymbol": ["Ctrl+1 Ctrl+S"],
  "navigate.findAction": ["Ctrl+Q"],
  "navigate.gotoLine": ["Ctrl+G"],
  "navigate.declaration": ["F12"],
  "navigate.implementation": ["Ctrl+F12"],
  "navigate.back": ["Ctrl+-", "Button4"],
  "navigate.forward": ["Ctrl+Shift+-", "Button5"],
  "navigate.switcher": ["Ctrl+Tab"],
  "navigate.switcherBack": ["Ctrl+Shift+Tab"],
  "navigate.nextError": ["Ctrl+Shift+F12"],
  "navigate.callHierarchy": ["Ctrl+K Ctrl+T"],
  "navigate.toggleBookmark": ["Ctrl+K Ctrl+K"],
  "navigate.showBookmarks": ["Ctrl+K Ctrl+W"],
  "navigate.selectIn": ["Ctrl+[ S"],
  // Find
  "find.inFile": ["Ctrl+F"],
  "find.replaceInFile": ["Ctrl+H"],
  "find.next": ["F3"],
  "find.previous": ["Shift+F3"],
  "find.popup.nextResult": ["F3"],
  "find.popup.previousResult": ["Shift+F3"],
  "find.inPath": ["Ctrl+Shift+F"],
  "find.replaceInPath": ["Ctrl+Shift+H"],
  "find.usages": ["Shift+F12"],
  "find.showUsages": ["Shift+Alt+F12"],
  // Editing
  "code.completion": ["Ctrl+Space", "Ctrl+J"],
  "code.parameterInfo": ["Ctrl+Shift+Space"],
  "code.quickDocumentation": ["Ctrl+K Ctrl+I"],
  "code.quickDefinition": ["Alt+F12"],
  "code.showIntentions": ["Ctrl+.", "Alt+Enter"],
  "code.reformat": ["Ctrl+K Ctrl+D", "Ctrl+K Ctrl+F"],
  "code.commentLine": ["Ctrl+K Ctrl+C", "Ctrl+K Ctrl+U", "Ctrl+/"],
  "code.duplicateLine": ["Ctrl+D", "Ctrl+E Ctrl+V"],
  "code.deleteLine": ["Ctrl+Shift+L"],
  "code.moveLineUp": ["Alt+Up"],
  "code.moveLineDown": ["Alt+Down"],
  "code.extendSelection": ["Shift+Alt+="],
  "code.shrinkSelection": ["Shift+Alt+-"],
  "code.fold": ["Ctrl+M Ctrl+M"],
  "code.foldAll": ["Ctrl+M Ctrl+O"],
  "code.unfoldAll": ["Ctrl+M Ctrl+L"],
  "code.surroundWith": ["Ctrl+K Ctrl+S"],
  "code.insertLiveTemplate": ["Ctrl+K Ctrl+X"],
  "code.addCaretNextOccurrence": ["Shift+Alt+."],
  "code.selectAllOccurrences": ["Shift+Alt+;"],
  "code.toggleCase": ["Ctrl+Shift+U"],
  "editor.matchBrace": ["Ctrl+]"],
  // Refactoring
  "refactor.rename": ["Ctrl+R Ctrl+R", "F2"],
  "refactor.extractMethod": ["Ctrl+R Ctrl+M"],
  "refactor.changeSignature": ["Ctrl+R Ctrl+O"],
  "refactor.extractVariable": ["Ctrl+R Ctrl+V"],
  "refactor.thisMenu": ["Ctrl+Shift+R"],
  // Files and windows
  "file.save": ["Ctrl+S"],
  "file.openProject": ["Ctrl+Shift+O"],
  "editor.closeTab": ["Ctrl+F4"],
  "toolwindow.closeContent": ["Ctrl+F4"],
  "editor.nextTab": ["Ctrl+Alt+PageDown"],
  "editor.prevTab": ["Ctrl+Alt+PageUp"],
  "window.toggleFullScreen": ["Shift+Alt+Enter"],
  // Build, run and debug
  "build.buildProject": ["Ctrl+Shift+B"],
  "run.run": ["Ctrl+F5"],
  "run.debug": ["F5"],
  "debug.resume": ["F5"],
  "run.stop": ["Shift+F5"],
  "run.rerun": ["Ctrl+Shift+F5"],
  "debug.toggleBreakpoint": ["F9"],
  "debug.viewBreakpoints": ["Ctrl+Alt+B"],
  "debug.stepOver": ["F10"],
  "debug.stepInto": ["F11"],
  "debug.stepOut": ["Shift+F11"],
  "debug.runToCursor": ["Ctrl+F10"],
  "debug.attachToProcess": ["Ctrl+Alt+P"],
  "debug.evaluate": ["Shift+F9"],
};

const MODIFIER_ORDER = ["Meta", "Ctrl", "Alt", "Shift"];

/** `Shift+Ctrl+f` and `Ctrl+Shift+F` alike (`keymap.ts` imports this module, so not its `normalizeBinding`). */
function canonical(binding: string): string {
  return binding
    .split(" ")
    .map((stroke) => {
      const parts = stroke.split("+");
      const key = parts.pop()!;
      const mods = MODIFIER_ORDER.filter((m) => parts.includes(m));
      return [...mods, key.length === 1 ? key.toUpperCase() : key].join("+");
    })
    .join(" ");
}

/** Visual Studio's keys for tool windows, kept next to IDEA's numbered ones (Alt+1…). */
export const visualStudioAdditions: Record<string, string[]> = {
  "toolwindow.project": ["Ctrl+Alt+L"],
  "toolwindow.problems": ["Ctrl+\\ E"],
  "toolwindow.run": ["Ctrl+Alt+O"],
  "toolwindow.terminal": ["Ctrl+`"],
  "view.zoomIn": ["Ctrl+Shift+."],
  "view.zoomOut": ["Ctrl+Shift+,"],
  // Alt+Down moves the line in the editor; in a search field it opens the history, as in
  // Visual Studio's own Find dropdowns.
  "find.popup.recentSearches": ["Alt+Down"],
  "find.field.recentSearches": ["Alt+Down"],
};

/**
 * `parent` with `overrides` replacing an action's keys and `additions` added to
 * them; a key either binds is removed from every other action.
 */
export function overlay(
  parent: Record<string, string[]>,
  overrides: Record<string, string[]>,
  additions: Record<string, string[]> = {},
): Record<string, string[]> {
  const taken = new Set([...Object.values(overrides), ...Object.values(additions)].flat().map(canonical));
  const out: Record<string, string[]> = {};
  for (const [id, keys] of Object.entries(parent))
    out[id] = overrides[id] ?? keys.filter((k) => !taken.has(canonical(k)) || additions[id]?.includes(k));
  for (const [id, keys] of Object.entries(overrides)) out[id] ??= keys;
  for (const [id, keys] of Object.entries(additions))
    out[id] = [...(out[id] ?? []), ...keys.filter((k) => !out[id]?.includes(k))];
  return out;
}

export const visualStudioWindows = overlay(intellijWindows, visualStudioOverrides, visualStudioAdditions);
