/** Everything that needs xterm.js; loaded on first use through `loader.ts`. */
export {
  activeSession,
  applySettingsToAll,
  applyThemeToAll,
  closeAll,
  closePane,
  newTab,
  newTabAt,
  requestClosePane,
  sessionForTerminalId,
  sessionFromElement,
  splitActive,
  startDirectory,
  useTerminals,
} from "./store";
export { TerminalTabs, TerminalView } from "./TerminalView";
export { applyThemeToEmbedded, createEmbeddedTerminal } from "./embedded";
export { closeTerminalFind, openTerminalFind, useTerminalFind } from "./FindBar";
