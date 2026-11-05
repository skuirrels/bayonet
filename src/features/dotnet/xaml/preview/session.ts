/**
 * One Avalonia previewer session per mounted preview: started with the
 * editor's text, sent each later text, cancelled on unmount. `generation`
 * restarts it (after a build of code-behind or view models).
 */
import { useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { commands, type PreviewEvent, type PreviewPhase, type RequestId } from "@/bindings";
import { call, isCancelled, isTauri, nextRequestId } from "@/platform/ipc";

export interface PreviewFrame {
  src: string;
  /** Device pixels. */
  width: number;
  height: number;
  scale: number;
}

export interface PreviewState {
  phase: PreviewPhase | "idle";
  /** Why the preview stopped or failed. */
  message: string | null;
  frame: PreviewFrame | null;
  /** The current text does not load; `frame` is the last good one. 0-based position. */
  error: { message: string; line: number | null; column: number | null } | null;
}

const IDLE: PreviewState = { phase: "idle", message: null, frame: null, error: null };

export function reduce(state: PreviewState, e: PreviewEvent): PreviewState {
  switch (e.kind) {
    case "phase":
      return { ...state, phase: e.phase, message: e.message };
    case "frame":
      return {
        ...state,
        frame: { src: `data:image/png;base64,${e.png}`, width: e.width, height: e.height, scale: e.scale ?? 1 },
      };
    case "xamlError":
      return { ...state, error: { message: e.message, line: e.line, column: e.column } };
    case "xamlLoaded":
      return { ...state, error: null };
  }
}

export function useXamlPreview(path: string, text: string | undefined, generation: number): PreviewState {
  const [state, setState] = useState<PreviewState>(IDLE);
  const request = useRef<RequestId | null>(null);
  const sent = useRef<string | undefined>(undefined);
  const latest = useRef(text);
  const ready = text !== undefined;

  useEffect(() => {
    latest.current = text;
    const id = request.current;
    if (id == null || text === undefined || text === sent.current) return;
    sent.current = text;
    void call(commands.dotnetXamlPreviewUpdate(id, text)).catch(() => undefined);
  }, [text]);

  useEffect(() => {
    if (!ready || !isTauri()) return;
    const id = nextRequestId();
    const channel = new Channel<PreviewEvent>();
    let live = true;
    channel.onmessage = (e) => {
      if (live) setState((s) => reduce(s, e));
    };
    request.current = id;
    sent.current = latest.current;
    void call(commands.dotnetXamlPreview(id, path, latest.current ?? "", window.devicePixelRatio || 1, channel)).catch(
      (e: unknown) => {
        if (live && !isCancelled(e))
          setState((s) => ({ ...s, phase: "failed", message: e instanceof Error ? e.message : String(e) }));
      },
    );
    return () => {
      live = false;
      request.current = null;
      void commands.cancelRequest(id);
      setState(IDLE);
    };
  }, [path, generation, ready]);

  return state;
}
