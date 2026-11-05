import { describe, expect, it } from "vitest";
import {
  bindingPathAt,
  clrNamespaces,
  dataTypeAt,
  elementType,
  enclosingElements,
  leadingSegments,
  namedType,
  typeNameAt,
  typeRef,
} from "./model";

const AVALONIA = `<Window xmlns="https://github.com/avaloniaui"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        xmlns:vm="using:App.ViewModels"
        xmlns:m="clr-namespace:App.Models;assembly=App.Core"
        x:Class="App.Views.MainWindow"
        x:DataType="vm:MainViewModel">
  <!-- <Ignored x:DataType="vm:Nope"> -->
  <StackPanel>
    <TextBlock Text="{Binding Owner.Na}" />
    <ListBox ItemsSource="{Binding Items}">
      <ListBox.ItemTemplate>
        <DataTemplate x:DataType="m:Item">
          <TextBlock Text="{Binding Path=Title, Mode=OneWay}" />
        </DataTemplate>
      </ListBox.ItemTemplate>
    </ListBox>
    <Button Command="{Binding SaveCommand}" Content="{x:Bind Title}" />
  </StackPanel>
</Window>`;

const at = (needle: string, offset = needle.length) => AVALONIA.indexOf(needle) + offset;

describe("XAML bindings (CI-16)", () => {
  it("maps xmlns prefixes to CLR namespaces", () => {
    const ns = clrNamespaces(AVALONIA);
    expect(Object.fromEntries(ns)).toEqual({ vm: "App.ViewModels", m: "App.Models" });
    expect(typeRef("vm:MainViewModel", ns)).toEqual({ namespace: "App.ViewModels", name: "MainViewModel" });
    expect(typeRef("{x:Type m:Item}", ns)).toEqual({ namespace: "App.Models", name: "Item" });
    expect(typeRef("App.Views.MainWindow", ns)).toEqual({ namespace: "App.Views", name: "MainWindow" });
    expect(typeRef("Foo", ns)).toEqual({ namespace: null, name: "Foo" });
  });

  it("finds the data type of the nearest element that sets one", () => {
    expect(dataTypeAt(AVALONIA, at("Owner.Na"))).toEqual({ namespace: "App.ViewModels", name: "MainViewModel" });
    expect(dataTypeAt(AVALONIA, at("Path=Ti"))).toEqual({ namespace: "App.Models", name: "Item" });
    // Back outside the template.
    expect(dataTypeAt(AVALONIA, at("SaveComm"))).toEqual({ namespace: "App.ViewModels", name: "MainViewModel" });
    expect(dataTypeAt(AVALONIA, at("x:Bind Ti"), true)).toEqual({ namespace: "App.Views", name: "MainWindow" });
    expect(enclosingElements(AVALONIA, at("Owner.Na")).map((e) => e.name)).toEqual([
      "Window",
      "StackPanel",
      "TextBlock",
    ]);
  });

  it("reads WPF design-time contexts and DataTemplate types", () => {
    const wpf = `<UserControl xmlns:d="http://schemas.microsoft.com/expression/blend/2008" xmlns:local="clr-namespace:Shop"
  d:DataContext="{d:DesignInstance Type=local:CartViewModel, IsDesignTimeCreatable=True}">
  <ItemsControl><ItemsControl.Resources><DataTemplate DataType="{x:Type local:Line}"><TextBlock Text="{Binding Qty}"/></DataTemplate></ItemsControl.Resources></ItemsControl>
  <TextBlock Text="{Binding Total}"/>
</UserControl>`;
    expect(dataTypeAt(wpf, wpf.indexOf("Total"))).toEqual({ namespace: "Shop", name: "CartViewModel" });
    expect(dataTypeAt(wpf, wpf.indexOf("Qty"))).toEqual({ namespace: "Shop", name: "Line" });
  });

  it("finds the binding path being typed", () => {
    expect(bindingPathAt(AVALONIA, at("Owner.Na"))).toEqual({ path: "Owner.Na", from: at("Owner."), xBind: false });
    expect(bindingPathAt(AVALONIA, at("{Binding "))).toMatchObject({ path: "", from: at("{Binding ") });
    expect(bindingPathAt(AVALONIA, at("Path=Ti"))?.path).toBe("Ti");
    expect(bindingPathAt(AVALONIA, at("Mode=One"))).toBeNull();
    expect(bindingPathAt(AVALONIA, at("x:Bind Ti"))).toMatchObject({ path: "Ti", xBind: true });
    expect(bindingPathAt(AVALONIA, at("<StackPanel"))).toBeNull();
    expect(leadingSegments("Items[0].Owner.Na")).toEqual(["Items[0]", "Owner"]);
  });

  it("follows collection and nullable member types", () => {
    expect(elementType("ObservableCollection<Item>?")).toBe("Item");
    expect(elementType("Dictionary<string, List<int>>")).toBe("List<int>");
    expect(elementType("Item[]")).toBe("Item");
    expect(namedType("Models.User?")).toEqual({ namespace: "Models", name: "User" });
    expect(namedType("List<int>")).toEqual({ namespace: null, name: "List" });
  });

  it("names the type under the caret in tags and type attributes", () => {
    const text = `<Grid xmlns:local="clr-namespace:App.Controls" xmlns:vm="using:App.ViewModels"><local:Card x:DataType="vm:CardViewModel"/></Grid>`;
    expect(typeNameAt(text, text.indexOf("Card ") + 2)).toEqual({ namespace: "App.Controls", name: "Card" });
    expect(typeNameAt(text, text.indexOf("CardViewModel") + 3)).toEqual({
      namespace: "App.ViewModels",
      name: "CardViewModel",
    });
    expect(typeNameAt(text, text.indexOf("<Grid") + 2)).toBeNull();
  });
});
