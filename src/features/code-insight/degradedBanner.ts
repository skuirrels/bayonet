/**
 * NFR-4: the notice above a file whose language server crashed. Editing and saving go
 * on (the Rust core owns the text); the server restarts on its own, is given the
 * editor's text again, and the notice stays until it is running.
 */
import { create } from "zustand";
import type { LanguageInfo } from "@/bindings";
import { commands, isTauri } from "@/platform/ipc";
import { registerEditorBanner, type EditorBanner } from "@/workbench/contributions";
import { restartServer, useServers, type ServerEntry } from "./servers";
import { tx } from "@/platform/i18n";

/** Lower-case extension or exact file name → language id. */
export const useFileLanguages = create<{ byName: ReadonlyMap<string, string> }>(() => ({ byName: new Map() }));

export function indexLanguages(languages: readonly Pick<LanguageInfo, "id" | "extensions" | "filenames">[]) {
  const byName = new Map<string, string>();
  for (const l of languages) {
    for (const f of l.filenames ?? []) if (!/[*?]/.test(f) && !byName.has(`/${f}`)) byName.set(`/${f}`, l.id);
    for (const e of l.extensions) if (!byName.has(e.toLowerCase())) byName.set(e.toLowerCase(), l.id);
  }
  useFileLanguages.setState({ byName });
}

export function languageOfPath(path: string, byName: ReadonlyMap<string, string>): string | null {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const exact = byName.get(`/${name}`);
  if (exact) return exact;
  const lower = name.toLowerCase();
  // Longest suffix first: `d.ts` before `ts`.
  for (let i = lower.indexOf("."); i >= 0; i = lower.indexOf(".", i + 1)) {
    const id = byName.get(lower.slice(i + 1));
    if (id) return id;
  }
  return null;
}

/** The server whose crash leaves `language` without code insight: none of its servers is running. */
export function degradedServer(language: string, servers: Iterable<ServerEntry>): ServerEntry | null {
  const serving = [...servers].filter((s) => s.languages.includes(language));
  if (serving.some((s) => s.status === "ready" || s.status === "busy")) return null;
  return serving.find((s) => s.status === "crashed") ?? serving.find((s) => s.afterCrash) ?? null;
}

export function degradedBanner(server: ServerEntry): EditorBanner {
  if (server.status === "crashed")
    return {
      text: tx`${server.name} crashed. Completion, navigation and inspections are unavailable until it runs again; editing and saving still work.`,
      level: "warning",
      actions: [{ label: "Restart", run: () => void restartServer(server.id) }],
    };
  return {
    text: tx`${server.name} is restarting after a crash. Code insight resumes when it is ready; editing and saving still work.`,
    level: "info",
    icon: "refresh",
  };
}

export function installDegradedBanner(): () => void {
  if (isTauri())
    void commands
      .languages()
      .then(indexLanguages)
      .catch(() => undefined);
  return registerEditorBanner({
    id: "code-insight.degraded",
    order: -2,
    useBanner: (path) => {
      const language = useFileLanguages((s) => languageOfPath(path, s.byName));
      const server = useServers((s) => (language ? degradedServer(language, s.servers.values()) : null));
      return server ? degradedBanner(server) : null;
    },
  });
}
