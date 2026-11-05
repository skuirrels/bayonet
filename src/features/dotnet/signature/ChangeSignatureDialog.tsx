/**
 * The Change Signature dialog for C# (CI-6): the parameters as editable rows
 * (declaration, and the value callers pass for a new one), moved with the
 * arrows, with a preview of the new signature.
 */
import { useState } from "react";
import clsx from "clsx";
import { create } from "zustand";
import type { MethodSignature, NewParameter } from "@/bindings";
import { t, tx } from "@/platform/i18n";
import { Dialog, IconButton, Input } from "@/ui";
import { initialRows, moveRow, preview, rowsError, toChange, unchanged, type SignatureRow } from "./model";
import styles from "./ChangeSignatureDialog.module.css";

interface Request {
  id: number;
  sig: MethodSignature;
  resolve: (change: NewParameter[] | null) => void;
}

const useRequest = create<{ request: Request | null }>(() => ({ request: null }));
let nextId = 1;

/** Asks for the new parameters of `sig`; `null` when cancelled or unchanged. */
export function askSignatureChange(sig: MethodSignature): Promise<NewParameter[] | null> {
  useRequest.getState().request?.resolve(null);
  return new Promise((resolve) => {
    const id = nextId++;
    useRequest.setState({
      request: {
        id,
        sig,
        resolve: (change) => {
          if (useRequest.getState().request?.id === id) useRequest.setState({ request: null });
          resolve(change);
        },
      },
    });
  });
}

export function ChangeSignatureHost() {
  const request = useRequest((s) => s.request);
  return request ? <ChangeSignatureDialog key={request.id} request={request} /> : null;
}

function kindLabel(kind: string): string {
  if (kind === "constructor") return tx`constructor`;
  if (kind === "localFunction") return tx`local function`;
  if (kind === "delegate") return tx`delegate`;
  return tx`method`;
}

function ChangeSignatureDialog({ request }: { request: Request }) {
  const { sig, resolve } = request;
  const [rows, setRows] = useState<SignatureRow[]>(() => initialRows(sig));
  const [selected, setSelected] = useState(0);
  const [nextKey, setNextKey] = useState(sig.parameters.length);
  const error = rowsError(sig, rows);
  const same = unchanged(sig, rows);
  const update = (i: number, patch: Partial<SignatureRow>) =>
    setRows(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const add = () => {
    const at = rows.length ? selected + 1 : 0;
    setRows([...rows.slice(0, at), { key: nextKey, from: null, text: "", value: "" }, ...rows.slice(at)]);
    setNextKey(nextKey + 1);
    setSelected(at);
  };
  const remove = () => {
    setRows(rows.filter((_, i) => i !== selected));
    setSelected(Math.max(0, Math.min(selected, rows.length - 2)));
  };
  const move = (delta: -1 | 1) => {
    setRows(moveRow(rows, selected, delta));
    setSelected(Math.max(0, Math.min(rows.length - 1, selected + delta)));
  };
  const kind = kindLabel(sig.kind);
  const name = sig.container && sig.kind !== "constructor" ? `${sig.container}.${sig.name}` : sig.name;
  return (
    <Dialog
      open
      title={tx`Change Signature`}
      onClose={() => resolve(null)}
      width={640}
      resizable="both"
      buttons={[
        {
          label: tx`Refactor`,
          role: "default",
          disabled: !!error || same,
          title: error ?? undefined,
          onClick: () => resolve(toChange(rows)),
        },
        { label: t("common.cancel"), role: "cancel" },
      ]}
    >
      <div className={styles.body}>
        <div className={styles.subject}>{tx`Change the signature of ${kind} '${name}'`}</div>
        <div className={styles.table} role="grid" aria-label={tx`Parameters`}>
          <div className={styles.toolbar}>
            <IconButton icon="add" title={tx`Add Parameter`} onClick={add} />
            <IconButton icon="minus" title={tx`Remove Parameter`} onClick={remove} disabled={!rows.length} />
            <IconButton icon="arrow-up" title={tx`Move Up`} onClick={() => move(-1)} disabled={selected <= 0} />
            <IconButton
              icon="arrow-down"
              title={tx`Move Down`}
              onClick={() => move(1)}
              disabled={selected >= rows.length - 1}
            />
          </div>
          <div className={styles.head} role="row">
            <span role="columnheader">{tx`Parameter`}</span>
            <span role="columnheader">{tx`Value for callers`}</span>
          </div>
          <div className={styles.rows}>
            {rows.length === 0 && <div className={styles.empty}>{tx`No parameters`}</div>}
            {rows.map((row, i) => (
              <div
                key={row.key}
                role="row"
                aria-selected={i === selected}
                className={clsx(styles.row, i === selected && styles.selected)}
                onFocus={() => setSelected(i)}
                onClick={() => setSelected(i)}
              >
                <Input
                  mono
                  data-autofocus={i === 0 ? "" : undefined}
                  aria-label={tx`Parameter declaration`}
                  placeholder={tx`Type name = default`}
                  value={row.text}
                  onChange={(e) => update(i, { text: e.target.value })}
                />
                {row.from === null ? (
                  <Input
                    mono
                    aria-label={tx`Value for callers`}
                    placeholder={tx`Value at call sites`}
                    value={row.value}
                    onChange={(e) => update(i, { value: e.target.value })}
                  />
                ) : (
                  <span className={styles.existing}>{tx`Existing parameter`}</span>
                )}
              </div>
            ))}
          </div>
        </div>
        <div className={styles.previewLabel}>{tx`Signature preview:`}</div>
        <pre className={styles.preview}>{preview(sig, rows)}</pre>
        {error && <div className={styles.error}>{error}</div>}
      </div>
    </Dialog>
  );
}
