// How Bayonet is released (scripts/release-kit, see RELEASING.md).
export default {
  // ~/.tauri/bayonet-updater.key, the WSL build clone and temporary folders.
  slug: "bayonet",
  siteRepo: "https://github.com/skuirrels/bayonet-releases.git",
  siteUrl: "https://skuirrels.github.io/bayonet-releases/",
  combinedFeed: "latest.json",
  // Files whose version follows src-tauri/tauri.conf.json (`make bump` sets them all, and Cargo.lock).
  // The app reports, and remote development downloads, the Cargo workspace version.
  versionFiles: ["package.json", "Cargo.toml"],
  icon: "src-tauri/icons/source/bayonet.svg",
  page: {
    description: "An open-source .NET IDE.",
    tagline:
      "An open-source IDE for C#, F# and the rest of .NET. Roslyn code intelligence, solution-wide refactorings, NuGet, unit tests and a debugger, in a fast native app.",
    license: "free and open source (Apache-2.0)",
    accent: ["#6e3fd6", "#a084f0"],
    features: [
      [
        "Solutions and projects",
        "Opens .sln, .slnx and .slnf files, with solution folders, dependencies and central package management.",
      ],
      [
        "Roslyn code intelligence",
        "Completion, navigation, find usages, inlay hints and analyser diagnostics as you type.",
      ],
      ["Refactorings", "Rename, extract, inline, change signature and move type to file across the solution."],
      ["NuGet", "Search feeds, install, update and remove packages per project or solution, including private feeds."],
      ["Unit tests", "Discover, run and debug xUnit, NUnit and MSTest tests, with results in a tree."],
      ["Debugger", "Breakpoints, stepping, watches and attach to process, with netcoredbg."],
    ],
    footer:
      "Apache-2.0 License · Bayonet is an independent project, ported from OpenIDEA, and is not affiliated with JetBrains or Microsoft.",
  },
};
