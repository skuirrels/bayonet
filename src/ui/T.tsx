import { useTx } from "@/platform/i18n";

/** English UI text, translated through the source-text catalog; re-renders on locale change. */
export function T({ children }: { children: string }) {
  return useTx()(children);
}
