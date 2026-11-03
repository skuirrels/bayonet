/** Status bar widget: the target framework the active C# / VB file is analysed for. */
import { useEffect, useRef, useState } from "react";
import { useEditorRegistry } from "@/editor/api";
import { onIdeEvent } from "@/platform/ipc";
import { tx } from "@/platform/i18n";
import { BarButton, Menu, type MenuItem } from "@/ui";
import { ROSLYN_LANGUAGES, chooseContext, frameworkOf, hasChoice, loadContexts, useFrameworks } from "./frameworks";

/** Roslyn loads projects after it is ready, and refreshes editors once it has. */
const RELOAD_DELAY = 300;

export function FrameworkStatus() {
  const active = useEditorRegistry((s) => s.active);
  const contexts = useFrameworks((s) => s.contexts);
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const ctx = active?.ctx;
  const doc = ctx && ROSLYN_LANGUAGES.has(ctx.language) && !ctx.virtual ? ctx.doc : null;
  const path = doc === null ? null : (ctx?.path ?? null);
  useEffect(() => {
    void loadContexts(doc, path);
    if (doc === null) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const reload = () => {
      clearTimeout(timer);
      timer = setTimeout(() => void loadContexts(doc, path), RELOAD_DELAY);
    };
    const offs = [
      onIdeEvent("lspRefresh", reload),
      onIdeEvent("languageServer", (e) => {
        if (e.status === "ready") reload();
      }),
    ];
    return () => {
      clearTimeout(timer);
      offs.forEach((off) => off());
    };
  }, [doc, path]);
  if (!hasChoice(contexts)) return null;
  const current = contexts.contexts[contexts.active] ?? contexts.contexts[0]!;
  const items: MenuItem[] = [
    { kind: "header", label: "Target Framework" },
    ...contexts.contexts.map((c, i): MenuItem => ({
      label: c.label,
      verbatim: true,
      checked: i === contexts.active,
      run: () => void chooseContext(c),
    })),
  ];
  const framework = frameworkOf(current.label);
  return (
    <>
      <BarButton
        ref={anchor}
        tooltip={current.label}
        aria-label={tx`Target framework: ${framework}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        {framework}
      </BarButton>
      <Menu
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        placement="top-end"
        items={items}
        aria-label="Target Framework"
      />
    </>
  );
}
