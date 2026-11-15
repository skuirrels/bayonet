import { describe, expect, it } from "vitest";
import type { XamlTypes } from "@/bindings";
import { catalog, membersOf, prefixes, resolveElement, xamlOptions, xamlSiteAt } from "./elements";

const ALIGN = ["Stretch", "Left", "Center", "Right"];
const TYPES: XamlTypes = {
  namespaces: [
    { xmlns: "https://github.com/avaloniaui", clrNamespace: "Avalonia.Controls" },
    { xmlns: "https://github.com/avaloniaui", clrNamespace: "Avalonia.Layout" },
  ],
  types: [
    {
      namespace: "Avalonia.Layout",
      name: "Layoutable",
      base: null,
      abstract: false,
      properties: [{ name: "HorizontalAlignment", type: "Avalonia.Layout.HorizontalAlignment", values: ALIGN }],
      events: [],
      attached: [],
    },
    {
      namespace: "Avalonia.Controls",
      name: "Control",
      base: "Avalonia.Layout.Layoutable",
      abstract: true,
      properties: [{ name: "Tag", type: "System.Object", values: null }],
      events: ["Loaded"],
      attached: [],
    },
    {
      namespace: "Avalonia.Controls",
      name: "Button",
      base: "Avalonia.Controls.Control",
      abstract: false,
      properties: [{ name: "IsDefault", type: "System.Boolean", values: ["True", "False"] }],
      events: ["Click"],
      attached: [],
    },
    {
      namespace: "Avalonia.Controls",
      name: "Grid",
      base: "Avalonia.Controls.Control",
      abstract: false,
      properties: [{ name: "RowDefinitions", type: "Avalonia.Controls.RowDefinitions", values: null }],
      events: [],
      attached: [{ name: "Row", type: "System.Int32", values: null }],
    },
    {
      namespace: "App.Views",
      name: "Card",
      base: "Avalonia.Controls.Control",
      abstract: false,
      properties: [{ name: "Title", type: "System.String", values: null }],
      events: [],
      attached: [],
    },
  ],
};

const HEAD = `<UserControl xmlns="https://github.com/avaloniaui" xmlns:local="clr-namespace:App.Views">\n  <Grid>\n    `;
const at = (text: string) => {
  const pos = text.indexOf("|");
  return { text: text.replace("|", ""), pos };
};
const labels = (s: string) => {
  const { text, pos } = at(s);
  const site = xamlSiteAt(text, pos);
  return site ? xamlOptions(text, site, catalog(TYPES)).map((o) => o.label) : null;
};

describe("XAML element completion", () => {
  it("resolves prefixes through xmlns definitions and clr-namespace", () => {
    const cat = catalog(TYPES);
    const scope = prefixes(HEAD, cat);
    expect(scope.get("")).toEqual(["Avalonia.Controls", "Avalonia.Layout"]);
    expect(resolveElement("local:Card", scope, cat)?.name).toBe("Card");
    expect(resolveElement("Card", scope, cat)).toBeNull();
    const button = resolveElement("Button", scope, cat)!;
    const members = membersOf(button, cat);
    expect(members.properties.map((p) => p.name)).toEqual(["IsDefault", "Tag", "HorizontalAlignment"]);
    expect(members.events).toEqual(["Click", "Loaded"]);
  });

  it("finds what is being typed", () => {
    const site = (s: string) => {
      const { text, pos } = at(s);
      return xamlSiteAt(text, pos);
    };
    expect(site(`${HEAD}<Bu|`)).toEqual({ kind: "element", from: HEAD.length + 1, parent: "Grid" });
    expect(site(`${HEAD}<Button Cl|`)?.kind).toBe("attribute");
    expect(site(`${HEAD}<Button HorizontalAlignment="Ce|`)).toMatchObject({
      kind: "value",
      attribute: "HorizontalAlignment",
    });
    expect(site(`${HEAD}<Button Content="{Binding Na|`)).toBeNull();
    expect(site(`${HEAD}<Button/>te|`)).toBeNull();
    expect(site(`${HEAD}</Gr|`)).toBeNull();
    expect(site(`${HEAD}<!-- <Bu|`)).toBeNull();
  });

  it("offers concrete elements in scope and property elements of the parent", () => {
    const options = labels(`${HEAD}<|`)!;
    expect(options).toEqual(
      expect.arrayContaining(["Button", "Grid", "Layoutable", "local:Card", "Grid.RowDefinitions"]),
    );
    expect(options).not.toContain("Control");
  });

  it("offers properties, events and attached properties as attributes", () => {
    const options = labels(`${HEAD}<Button |`)!;
    expect(options).toEqual(
      expect.arrayContaining(["IsDefault", "HorizontalAlignment", "Click", "Loaded", "Grid.Row"]),
    );
    expect(labels(`${HEAD}<local:Card |`)).toContain("Title");
  });

  it("offers enum and bool values", () => {
    expect(labels(`${HEAD}<Button HorizontalAlignment="|`)).toEqual(ALIGN);
    expect(labels(`${HEAD}<Button IsDefault="|`)).toEqual(["True", "False"]);
    expect(labels(`${HEAD}<Button Grid.Row="|`)).toEqual([]);
  });
});
