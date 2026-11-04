import { describe, expect, it } from "vitest";
import type { DbColumnInfo } from "@/platform/ipc";
import {
  alterTableSql,
  createTableSql,
  draftOf,
  dropSql,
  ident,
  reference,
  renameSql,
  scriptSql,
  truncateSql,
  type ColumnDraft,
} from "./objects";

const col = (name: string, dataType: string, extra: Partial<DbColumnInfo> = {}): DbColumnInfo => ({
  name,
  dataType,
  nullable: true,
  default: null,
  primaryKey: false,
  ...extra,
});

const users = [
  col("id", "INTEGER", { primaryKey: true, nullable: false }),
  col("email", "TEXT", { nullable: false }),
  col("name", "TEXT"),
];

describe("database object SQL", () => {
  it("quotes only when needed", () => {
    expect(ident("postgres", "users")).toBe("users");
    expect(ident("postgres", "Users")).toBe('"Users"');
    expect(ident("sqlite", "order")).toBe('"order"');
    expect(ident("mysql", "my table")).toBe("`my table`");
  });

  it("copies references", () => {
    expect(reference("sqlite", { kind: "table", schema: "main", name: "users" })).toBe("users");
    expect(reference("postgres", { kind: "column", schema: "public", name: "users", column: "email" })).toBe(
      "public.users.email",
    );
  });

  it("renames per dialect", () => {
    expect(renameSql("sqlite", { kind: "table", schema: "main", name: "users" }, "people")).toBe(
      "alter table users rename to people;",
    );
    expect(renameSql("mysql", { kind: "table", schema: "app", name: "users" }, "people")).toBe(
      "rename table app.users to app.people;",
    );
    expect(renameSql("postgres", { kind: "column", schema: "public", name: "users", column: "email" }, "mail")).toBe(
      "alter table public.users rename column email to mail;",
    );
  });

  it("drops with IF EXISTS / CASCADE", () => {
    const t = { kind: "table" as const, schema: "public", name: "users" };
    expect(dropSql("postgres", t, { ifExists: true, cascade: true })).toBe(
      "drop table if exists public.users cascade;",
    );
    expect(dropSql("sqlite", { ...t, schema: "main" }, { ifExists: false, cascade: true })).toBe("drop table users;");
    expect(
      dropSql(
        "mysql",
        { kind: "index", schema: "app", name: "ix", table: "users" },
        { ifExists: false, cascade: false },
      ),
    ).toBe("drop index ix on app.users;");
    expect(
      dropSql(
        "sqlite",
        { kind: "column", schema: "main", name: "users", column: "name" },
        { ifExists: true, cascade: false },
      ),
    ).toBe("alter table users drop column name;");
  });

  it("truncates (SQLite deletes)", () => {
    expect(truncateSql("sqlite", "main", "users", { restartIdentity: true, cascade: true })).toBe("delete from users;");
    expect(truncateSql("postgres", "public", "users", { restartIdentity: true, cascade: false })).toBe(
      "truncate table public.users restart identity;",
    );
  });

  it("writes script templates keyed by the primary key", () => {
    expect(scriptSql("sqlite", "selectFirst", "main", "users", users)).toBe("select *\nfrom users\nlimit 10;\n");
    expect(scriptSql("sqlite", "insert", "main", "users", users)).toBe(
      "insert into users (id, email, name)\nvalues (?, ?, ?);\n",
    );
    expect(scriptSql("sqlite", "update", "main", "users", users)).toBe(
      "update users\nset email = ?,\n    name = ?\nwhere id = ?;\n",
    );
    expect(scriptSql("sqlite", "delete", "main", "users", users)).toBe("delete\nfrom users\nwhere id = ?;\n");
  });

  it("creates tables", () => {
    const drafts: ColumnDraft[] = users.map(draftOf);
    expect(createTableSql("sqlite", "main", "people", drafts)).toBe(
      "create table people\n(\n    id INTEGER not null,\n    email TEXT not null,\n    name TEXT,\n    primary key (id)\n);\n",
    );
  });

  it("alters tables: SQLite renames / adds / drops, PostgreSQL also changes columns", () => {
    const d = users.map(draftOf);
    d[1] = { ...d[1]!, name: "mail" };
    d[2] = { ...d[2]!, dropped: true };
    d.push({
      key: 9,
      original: null,
      name: "age",
      dataType: "INTEGER",
      notNull: false,
      default: "0",
      primaryKey: false,
      dropped: false,
    });
    expect(alterTableSql("sqlite", "main", "users", "people", d)).toEqual([
      "alter table users rename column email to mail;",
      "alter table users drop column name;",
      "alter table users add column age INTEGER default 0;",
      "alter table users rename to people;",
    ]);
    const p = users.map(draftOf);
    p[2] = { ...p[2]!, dataType: "varchar(40)", notNull: true, default: "''" };
    expect(alterTableSql("postgres", "public", "users", "users", p)).toEqual([
      "alter table public.users alter column name type varchar(40);",
      "alter table public.users alter column name set not null;",
      "alter table public.users alter column name set default '';",
    ]);
    expect(alterTableSql("mysql", "app", "users", "users", p)).toEqual([
      "alter table app.users change column name name varchar(40) default '' not null;",
    ]);
  });
});
