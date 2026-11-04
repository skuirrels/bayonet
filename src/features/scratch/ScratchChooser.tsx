import { useMemo, useState, type KeyboardEvent } from "react";
import clsx from "clsx";
import { HighlightedText, Icon, List, Popup, fileIconName, listNavigate, T } from "@/ui";
import { chooserRows, type LanguageRow, type ScratchLanguage } from "./model";
import { closeChooser, rememberLanguage, useScratch } from "./store";
import { createScratch } from "./scratches";
import styles from "./ScratchChooser.module.css";

/** Overlay: IDEA's "New Scratch File" popup — languages, speed search, recent first. */
export function ScratchChooserHost() {
  const languages = useScratch((s) => s.languages);
  const seq = useScratch((s) => s.seq);
  if (!languages) return null;
  return <ScratchChooser key={seq} languages={languages} />;
}

function ScratchChooser({ languages }: { languages: readonly ScratchLanguage[] }) {
  const recent = useScratch((s) => s.recent);
  const [filter, setFilter] = useState("");
  const [active, setActive] = useState(0);
  const rows = useMemo(() => chooserRows(languages, recent, filter), [languages, recent, filter]);
  const current = Math.min(active, rows.length - 1);
  const anchor = useMemo(() => ({ x: Math.max(16, window.innerWidth / 2 - 140), y: window.innerHeight / 5 }), []);

  const pick = (row: LanguageRow | undefined) => {
    if (!row) return;
    closeChooser();
    rememberLanguage(row.lang.id);
    void createScratch(row.lang);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const next = listNavigate(e.key, { items: rows, active: current, wrap: true });
    if (next !== null) {
      e.preventDefault();
      setActive(next);
    } else if (e.key === "Enter") {
      e.preventDefault();
      pick(rows[current]);
    } else if (e.key === "Backspace") {
      e.preventDefault();
      setFilter((f) => f.slice(0, -1));
      setActive(0);
    } else if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      setFilter((f) => f + e.key);
      setActive(0);
    }
  };

  return (
    <Popup
      open
      anchor={anchor}
      placement="bottom-start"
      onClose={closeChooser}
      role="dialog"
      aria-label="New Scratch File"
      className={styles.popup}
    >
      <div className={styles.body} tabIndex={0} data-autofocus onKeyDown={onKeyDown}>
        <div className={styles.header}>
          <span className={styles.title}>
            <T>New Scratch File</T>
          </span>
          {filter && <span className={styles.filter}>{filter}</span>}
        </div>
        <List
          items={rows}
          getKey={(r) => r.lang.id}
          activeIndex={current}
          onActiveChange={setActive}
          onActivate={(r) => pick(r)}
          focusable={false}
          hoverSelects
          rowHeight={24}
          className={styles.list}
          aria-label="Languages"
          empty={
            <div className={styles.empty}>
              <T>No matching languages</T>
            </div>
          }
          renderItem={(r, state) => (
            <div className={clsx(styles.row, state.active && styles.active)} data-language={r.lang.id}>
              <Icon name={fileIconName(`scratch.${r.lang.ext}`)} size={16} />
              <span className={styles.label}>
                <HighlightedText text={r.lang.name} indices={r.matches ?? undefined} />
              </span>
            </div>
          )}
        />
      </div>
    </Popup>
  );
}
