import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AuditFinding, Diagnostic, PackageAudit } from "@/bindings";

const published = new Map<string, Diagnostic[]>();
vi.mock("@/editor/lsp/diagnostics", () => ({
  setFileDiagnostics: (path: string, _source: string, items: Diagnostic[]) => {
    if (items.length) published.set(path, items);
    else published.delete(path);
  },
}));

const { auditIssues, findingDiagnostics, publishAudit } = await import("./audit");

const range = (line: number) => ({ start: { line, col: 19 }, end: { line, col: 35 } });
const finding = (f: Partial<AuditFinding> & { id: string }): AuditFinding => ({
  version: "1.0.0",
  frameworks: ["net10.0"],
  transitive: false,
  vulnerabilities: [],
  deprecation: null,
  range: range(4),
  ...f,
});
const A = "/s/A/A.csproj";
const B = "/s/B/B.csproj";
const GHSA = (n: string) => `https://github.com/advisories/GHSA-${n}`;
const AUDIT: PackageAudit = {
  problems: [],
  projects: [
    {
      path: A,
      findings: [
        finding({
          id: "System.Text.Json",
          version: "8.0.0",
          vulnerabilities: [
            { severity: "Moderate", url: GHSA("a") },
            { severity: "High", url: GHSA("b") },
          ],
        }),
        finding({
          id: "Newtonsoft.Json",
          version: "10.0.2",
          transitive: true,
          vulnerabilities: [{ severity: "High", url: GHSA("c") }],
          range: range(0),
        }),
      ],
    },
    {
      path: B,
      findings: [
        finding({
          id: "WindowsAzure.Storage",
          version: "9.3.3",
          deprecation: { reasons: ["Legacy"], alternative: "Azure.Storage.Common" },
        }),
        finding({ id: "newtonsoft.json", version: "12.0.1", vulnerabilities: [{ severity: "High", url: GHSA("c") }] }),
      ],
    },
  ],
};

beforeEach(() => {
  publishAudit(null);
});

describe("package audit (NUG-4)", () => {
  it("groups issues by package, worst advisory first", () => {
    const issues = auditIssues(AUDIT);
    const json = issues.get("system.text.json")!;
    expect(json.worst).toBe("High");
    expect(json.vulnerabilities.map((a) => a.severity)).toEqual(["High", "Moderate"]);
    const newtonsoft = issues.get("newtonsoft.json")!;
    expect(newtonsoft.vulnerabilities).toHaveLength(1);
    expect(newtonsoft.projects.map((p) => [p.name, p.version, p.transitive])).toEqual([
      ["A", "10.0.2", true],
      ["B", "12.0.1", false],
    ]);
    expect(newtonsoft.transitive).toBe(false);
    expect(issues.get("windowsazure.storage")!.worst).toBeNull();
  });

  it("warns on the reference with NuGet's codes, and strikes deprecated packages", () => {
    const [moderate, high] = findingDiagnostics(AUDIT.projects[0]!.findings[0]!);
    expect(moderate!.code).toBe("NU1902");
    expect(high!.code).toBe("NU1903");
    expect(high!.message).toBe(
      `Package 'System.Text.Json' 8.0.0 has a known high severity vulnerability, ${GHSA("b")}`,
    );
    expect(high!.range.start.line).toBe(4);
    const [transitive] = findingDiagnostics(AUDIT.projects[0]!.findings[1]!);
    expect(transitive!.message).toMatch(/^Transitive package 'Newtonsoft.Json' 10.0.2/);
    const [deprecated] = findingDiagnostics(AUDIT.projects[1]!.findings[0]!);
    expect(deprecated).toMatchObject({ severity: "warning", deprecated: true });
    expect(deprecated!.message).toBe(
      "Package 'WindowsAzure.Storage' 9.3.3 is deprecated (legacy): use Azure.Storage.Common instead",
    );
  });

  it("clears project files that no longer have warnings", () => {
    publishAudit(AUDIT);
    expect([...published.keys()].sort()).toEqual([A, B]);
    publishAudit({ problems: [], projects: [AUDIT.projects[1]!] });
    expect([...published.keys()]).toEqual([B]);
    publishAudit(null);
    expect(published.size).toBe(0);
  });
});
