/**
 * Vulnerable and deprecated packages (NUG-4): `dotnet package list --vulnerable`
 * and `--deprecated` results, per package for the NuGet window and as warnings on
 * the `PackageReference` of each project file (the `Project` element for packages
 * that come in under another one).
 */
import type { Advisory, AuditFinding, Deprecation, Diagnostic, PackageAudit } from "@/bindings";
import { setFileDiagnostics } from "@/editor/lsp/diagnostics";
import { tx } from "@/platform/i18n";
import { projectName } from "./model";

/** Producer of the diagnostics (inspection profiles key on it). */
export const AUDIT_SOURCE = "nuget-audit";

const SEVERITY_ORDER = ["low", "moderate", "high", "critical"];
/** NuGet's own audit warning codes, by severity. */
const SEVERITY_CODE: Record<string, string> = {
  low: "NU1901",
  moderate: "NU1902",
  high: "NU1903",
  critical: "NU1904",
};

export const severityRank = (s: string) => SEVERITY_ORDER.indexOf(s.toLowerCase());

export function severityLabel(s: string): string {
  switch (s.toLowerCase()) {
    case "low":
      return tx`low`;
    case "moderate":
      return tx`moderate`;
    case "high":
      return tx`high`;
    case "critical":
      return tx`critical`;
    default:
      return s.toLowerCase();
  }
}

export function reasonLabel(r: string): string {
  switch (r) {
    case "Legacy":
      return tx`legacy`;
    case "CriticalBugs":
      return tx`critical bugs`;
    default:
      return tx`other`;
  }
}

/** What the audit says about one package id, over every project. */
export interface PackageIssues {
  id: string;
  /** Distinct advisories, worst first. */
  vulnerabilities: Advisory[];
  /** The worst severity, `null` when only deprecated. */
  worst: string | null;
  deprecation: Deprecation | null;
  /** Versions with issues, by project name. */
  projects: Array<{ project: string; name: string; version: string | null; transitive: boolean }>;
  /** No project references it: it comes in under other packages. */
  transitive: boolean;
}

/** Issues by lowercase package id. */
export function auditIssues(audit: PackageAudit | null): Map<string, PackageIssues> {
  const out = new Map<string, PackageIssues>();
  for (const p of audit?.projects ?? []) {
    for (const f of p.findings) {
      const key = f.id.toLowerCase();
      let issues = out.get(key);
      if (!issues) {
        issues = { id: f.id, vulnerabilities: [], worst: null, deprecation: null, projects: [], transitive: true };
        out.set(key, issues);
      }
      for (const a of f.vulnerabilities)
        if (!issues.vulnerabilities.some((o) => o.url === a.url)) issues.vulnerabilities.push(a);
      issues.deprecation ??= f.deprecation;
      issues.projects.push({
        project: p.path,
        name: projectName(p.path),
        version: f.version,
        transitive: f.transitive,
      });
      if (!f.transitive) issues.transitive = false;
    }
  }
  for (const issues of out.values()) {
    issues.vulnerabilities.sort((a, b) => severityRank(b.severity) - severityRank(a.severity));
    issues.worst = issues.vulnerabilities[0]?.severity ?? null;
    issues.projects.sort((a, b) => a.name.localeCompare(b.name));
  }
  return out;
}

/** The warnings of one finding. */
export function findingDiagnostics(f: AuditFinding): Diagnostic[] {
  const range = f.range ?? { start: { line: 0, col: 0 }, end: { line: 0, col: 0 } };
  const id = f.id;
  const version = f.version ?? "";
  const out: Diagnostic[] = f.vulnerabilities.map((a) => {
    const severity = severityLabel(a.severity);
    const url = a.url;
    return {
      range,
      severity: "warning",
      message: f.transitive
        ? tx`Transitive package '${id}' ${version} has a known ${severity} severity vulnerability, ${url}`
        : tx`Package '${id}' ${version} has a known ${severity} severity vulnerability, ${url}`,
      source: "NuGet",
      code: SEVERITY_CODE[a.severity.toLowerCase()] ?? null,
      server: AUDIT_SOURCE,
    };
  });
  if (f.deprecation) {
    const reasons = f.deprecation.reasons.map(reasonLabel).join(", ");
    const alternative = f.deprecation.alternative;
    out.push({
      range,
      severity: "warning",
      message: alternative
        ? tx`Package '${id}' ${version} is deprecated (${reasons}): use ${alternative} instead`
        : tx`Package '${id}' ${version} is deprecated (${reasons})`,
      source: "NuGet",
      code: null,
      deprecated: true,
      server: AUDIT_SOURCE,
    });
  }
  return out;
}

/** Project files given warnings last time, to clear when they have none. */
let published = new Set<string>();

/** Shows the audit as warnings in the project files (and Problems); `null` clears them. */
export function publishAudit(audit: PackageAudit | null) {
  const next = new Set<string>();
  for (const p of audit?.projects ?? []) {
    const items = p.findings.flatMap(findingDiagnostics);
    if (!items.length) continue;
    next.add(p.path);
    setFileDiagnostics(p.path, AUDIT_SOURCE, items);
  }
  for (const path of published) if (!next.has(path)) setFileDiagnostics(path, AUDIT_SOURCE, []);
  published = next;
}
