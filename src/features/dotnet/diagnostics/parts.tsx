/** Small pieces the Diagnostics tabs share. */
import { useMemo } from "react";
import { Button, Spinner, T } from "@/ui";
import { useSolution } from "../projects/store";
import { cancelJob, useDiagnostics, type Job } from "./store";
import styles from "./Diagnostics.module.css";

/** The assembly names of the solution's projects ("your code"). */
export function useUserModules(): ReadonlySet<string> {
  const projects = useSolution((s) => s.projects);
  return useMemo(() => new Set(projects.map((p) => p.name)), [projects]);
}

/** A spinner, `label` and Cancel while `job` runs. */
export function Busy({ job, label }: { job: Job; label: string }) {
  const running = useDiagnostics((s) => s.busy[job] != null);
  if (!running) return null;
  return (
    <span className={styles.busy}>
      <Spinner size={14} />
      {label}
      <Button size="small" variant="ghost" onClick={() => cancelJob(job)}>
        <T>Cancel</T>
      </Button>
    </span>
  );
}

/** A headline number with its label below. */
export function Figure({ label, value, delta }: { label: string; value: string; delta?: string }) {
  return (
    <div className={styles.figure}>
      <span className={styles.figureValue}>{value}</span>
      <span className={styles.figureLabel}>
        {label}
        {delta && <span className={styles.delta}> {delta}</span>}
      </span>
    </div>
  );
}
