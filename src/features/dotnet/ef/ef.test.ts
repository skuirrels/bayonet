import { describe, expect, it } from "vitest";
import { LATEST, REVERT_ALL, migrationNameError, updateMigration, updateTargets } from "./ef";

describe("Entity Framework Core helpers", () => {
  it("accepts C# identifiers as migration names", () => {
    expect(migrationNameError("InitialCreate")).toBeNull();
    expect(migrationNameError("_v2")).toBeNull();
    expect(migrationNameError("")).not.toBeNull();
    expect(migrationNameError("2Fast")).not.toBeNull();
    expect(migrationNameError("Add Orders")).not.toBeNull();
  });

  it("offers the latest, each migration newest first, then reverting all", () => {
    const targets = updateTargets(["InitialCreate", "AddOrders"]);
    expect(targets).toEqual([LATEST, "AddOrders", "InitialCreate", REVERT_ALL]);
    expect(targets.map(updateMigration)).toEqual([null, "AddOrders", "InitialCreate", "0"]);
  });
});
