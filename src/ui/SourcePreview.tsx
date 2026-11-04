/**
 * Read-only source view around a line, with line numbers and highlighted
 * ranges: the preview pane beside result trees (TODO, Problems, Inspection
 * Results…). The caller supplies the text loader (open buffer or disk).
 */
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import clsx from "clsx";
import { T } from "./T";
import styles from "./SourcePreview.module.css";
import { tx } from "@/platform/i18n";

export interface SourceHighlight {
  line: number;
  /** UTF-16 column ranges in the line. */
  ranges: ReadonlyArray<readonly [number, number]>;
}

/** Lines rendered on each side of the current one. */
const WINDOW = 300;

export interface SourcePreviewProps {
  path: string | null;
  /** 0-based line to centre and mark as current. */
  line: number;
  highlights?: readonly SourceHighlight[];
  /** Stable (module-level) loader; a new function reloads. */
  load: (path: string) => Promise<string>;
  /** Bump to reload the same path. */
  version?: number;
  className?: string;
}

export function SourcePreview({ path, line, highlights = [], load, version = 0, className }: SourcePreviewProps) {
  const [text, setText] = useState<{ path: string; lines: string[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const current = text && text.path === path ? text : null;

  useEffect(() => {
    if (!path) return;
    let alive = true;
    // Debounced so arrowing through results doesn't load every file.
    const t = setTimeout(() => {
      load(path).then(
        (content) => {
          if (!alive) return;
          setText({ path, lines: content.split(/\r\n|\r|\n/) });
          setError(null);
        },
        (e) => alive && setError(String((e as Error)?.message ?? e)),
      );
    }, 80);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [path, version, load]);

  useLayoutEffect(() => {
    const box = scrollRef.current;
    const el = box?.querySelector<HTMLElement>(`[data-line="${line}"]`);
    if (box && el) box.scrollTop = el.offsetTop - box.clientHeight / 2 + el.offsetHeight / 2;
  }, [current, line]);

  if (!path)
    return (
      <div className={clsx(styles.preview, styles.empty, className)}>
        <T>Select an item to preview</T>
      </div>
    );
  if (error && !current) return <div className={clsx(styles.preview, styles.empty, className)}>{error}</div>;
  if (!current) return <div className={clsx(styles.preview, className)} />;

  const byLine = new Map(highlights.map((h) => [h.line, h.ranges]));
  const start = Math.max(0, line - WINDOW);
  const end = Math.min(current.lines.length, line + WINDOW);
  const rows: ReactNode[] = [];
  for (let n = start; n < end; n++) {
    const content = current.lines[n] ?? "";
    const ranges = byLine.get(n);
    rows.push(
      <div key={n} data-line={n} className={clsx(styles.codeLine, n === line && styles.currentLine)}>
        <span className={styles.gutter}>{n + 1}</span>
        <span className={styles.code}>{ranges ? highlight(content, ranges) : content || " "}</span>
      </div>,
    );
  }
  return (
    <div ref={scrollRef} className={clsx(styles.preview, className)} role="region" aria-label={tx("Preview")}>
      <pre className={styles.pre}>{rows}</pre>
    </div>
  );
}

function highlight(text: string, ranges: ReadonlyArray<readonly [number, number]>): ReactNode[] {
  const parts: ReactNode[] = [];
  let pos = 0;
  [...ranges]
    .sort((a, b) => a[0] - b[0])
    .forEach(([s, e], i) => {
      if (s < pos || e <= s) return;
      if (s > pos) parts.push(text.slice(pos, s));
      parts.push(
        <mark key={i} className={styles.match}>
          {text.slice(s, e)}
        </mark>,
      );
      pos = e;
    });
  if (pos < text.length) parts.push(text.slice(pos));
  return parts;
}
