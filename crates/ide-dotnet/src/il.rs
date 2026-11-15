//! IL view (CI-10): which assembly to disassemble for a C# file.
//!
//! Go to Definition into a referenced assembly opens a file Roslyn wrote under
//! `<temp>/MetadataAsSource/`. Its header names the assembly the project compiles
//! against, which for the framework is a reference assembly: declarations without
//! bodies. The code that runs is in the shared runtime next to the `dotnet` host (or
//! in a package's `lib` folder), so that is disassembled when it can be found.
use std::path::{Path, PathBuf};

/// The assembly named by the header of a metadata source file:
///
/// ```text
/// #region Assembly System.Console, Version=10.0.0.0, Culture=neutral, PublicKeyToken=…
/// // /usr/share/dotnet/packs/Microsoft.NETCore.App.Ref/10.0.0/ref/net10.0/System.Console.dll
/// // Decompiled with ICSharpCode.Decompiler
/// #endregion
/// ```
///
/// `None` for a file without it (Source Link and embedded sources are the library's own
/// files) or where Roslyn did not know the location.
pub fn metadata_source_assembly(text: &str) -> Option<PathBuf> {
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    lines.next()?.strip_prefix("#region Assembly ")?;
    let path = Path::new(lines.next()?.strip_prefix("//")?.trim());
    let binary = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["dll", "exe", "winmd"].iter().any(|known| e.eq_ignore_ascii_case(known)));
    binary.then(|| path.to_path_buf())
}

/// The assemblies to look for a type in, best first, for code compiled against
/// `assembly`: its implementation where there is one, then `assembly` itself.
/// `dotnet_root` is the folder of the `dotnet` host, for reference packs that are not
/// next to a shared runtime (restored into the NuGet cache).
pub fn implementation_candidates(assembly: &Path, dotnet_root: Option<&Path>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> =
        runtime_assembly(assembly, dotnet_root).or_else(|| package_lib(assembly)).into_iter().collect();
    if !out.iter().any(|p| p == assembly) {
        out.push(assembly.to_path_buf());
    }
    out
}

/// `<pack>/<version>/ref/<tfm>/<file>` taken apart.
struct RefAssembly<'a> {
    /// The `<pack>` folder.
    pack: &'a Path,
    version: &'a str,
    tfm: &'a std::ffi::OsStr,
    file: &'a std::ffi::OsStr,
}

fn ref_assembly(assembly: &Path) -> Option<RefAssembly<'_>> {
    let file = assembly.file_name()?;
    let tfm_dir = assembly.parent()?;
    let ref_dir = tfm_dir.parent()?;
    if !ref_dir.file_name()?.eq_ignore_ascii_case("ref") {
        return None;
    }
    let version_dir = ref_dir.parent()?;
    Some(RefAssembly {
        pack: version_dir.parent()?,
        version: version_dir.file_name()?.to_str()?,
        tfm: tfm_dir.file_name()?,
        file,
    })
}

/// `System.Console.dll` of the shared runtime for the one in a targeting pack
/// (`Microsoft.NETCore.App.Ref` is `shared/Microsoft.NETCore.App`): the pack's own
/// version, else the newest patch of its `major.minor`.
fn runtime_assembly(assembly: &Path, dotnet_root: Option<&Path>) -> Option<PathBuf> {
    let r = ref_assembly(assembly)?;
    let pack = r.pack.file_name()?.to_str()?;
    let framework =
        pack.get(..pack.len().checked_sub(4)?).filter(|_| pack[pack.len() - 4..].eq_ignore_ascii_case(".ref"))?;
    let (major, minor, _) = version_of(r.version)?;
    // `<root>/packs/<pack>` sits beside `<root>/shared`.
    let beside = r.pack.parent().and_then(Path::parent);
    [beside, dotnet_root].into_iter().flatten().find_map(|root| {
        let shared = std::fs::read_dir(root.join("shared")).ok()?;
        let framework_dir = shared
            .flatten()
            .find(|e| e.file_name().to_str().is_some_and(|n| n.eq_ignore_ascii_case(framework)))?
            .path();
        let exact = framework_dir.join(r.version).join(r.file);
        if exact.is_file() {
            return Some(exact);
        }
        std::fs::read_dir(&framework_dir)
            .ok()?
            .flatten()
            .filter_map(|e| {
                let version = version_of(e.file_name().to_str()?)?;
                let file = e.path().join(r.file);
                (version.0 == major && version.1 == minor && file.is_file()).then_some((version, file))
            })
            .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
            .map(|(_, file)| file)
    })
}

/// A NuGet package's `lib/<tfm>/<file>` for its `ref/<tfm>/<file>`.
fn package_lib(assembly: &Path) -> Option<PathBuf> {
    let r = ref_assembly(assembly)?;
    Some(r.pack.join(r.version).join("lib").join(r.tfm).join(r.file)).filter(|p| p.is_file())
}

/// `10.0.3` and `11.0.0-rc.1.25451.107` as numbers; a missing patch is 0.
fn version_of(name: &str) -> Option<(u32, u32, u32)> {
    let mut parts = name.split(['-', '+']).next()?.split('.').map(str::parse::<u32>);
    let major = parts.next()?.ok()?;
    let minor = parts.next()?.ok()?;
    Some((major, minor, parts.next().and_then(Result::ok).unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }

    #[test]
    fn reads_the_assembly_from_a_decompiled_header() {
        // As Roslyn writes it, byte order mark included.
        let text = "\u{feff}#region Assembly System.Console, Version=11.0.0.0, Culture=neutral, PublicKeyToken=b03f5f7f11d50a3a\n// /usr/local/share/dotnet/packs/Microsoft.NETCore.App.Ref/11.0.0/ref/net11.0/System.Console.dll\n// Decompiled with ICSharpCode.Decompiler \n#endregion\n\nnamespace System;\n";
        assert_eq!(
            metadata_source_assembly(text),
            Some(PathBuf::from(
                "/usr/local/share/dotnet/packs/Microsoft.NETCore.App.Ref/11.0.0/ref/net11.0/System.Console.dll"
            ))
        );
        let windows = "#region Assembly Lib, Version=1.0.0.0\r\n// C:\\Users\\me\\.nuget\\packages\\lib\\1.0.0\\lib\\net8.0\\Lib.DLL\r\n#endregion\r\n";
        assert_eq!(
            metadata_source_assembly(windows),
            Some(PathBuf::from("C:\\Users\\me\\.nuget\\packages\\lib\\1.0.0\\lib\\net8.0\\Lib.DLL"))
        );
    }

    #[test]
    fn files_without_an_assembly_header_have_none() {
        assert_eq!(metadata_source_assembly("// Licensed to the .NET Foundation\nnamespace System;\n"), None);
        assert_eq!(
            metadata_source_assembly("#region Assembly Lib, Version=1.0.0.0\n// location unknown\n#endregion\n"),
            None
        );
        assert_eq!(metadata_source_assembly("#region Assembly Lib, Version=1.0.0.0"), None);
        assert_eq!(metadata_source_assembly(""), None);
    }

    #[test]
    fn a_targeting_pack_assembly_resolves_to_the_shared_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let reference = root.join("packs/Microsoft.NETCore.App.Ref/10.0.3/ref/net10.0/System.Console.dll");
        touch(&reference);
        // No runtime at all: only the reference assembly.
        assert_eq!(implementation_candidates(&reference, None), std::slice::from_ref(&reference));

        // Another major.minor is never used; of the same one, the newest patch.
        for version in ["9.0.6", "10.0.0", "10.0.11", "10.0.8", "11.0.0"] {
            touch(&root.join("shared/Microsoft.NETCore.App").join(version).join("System.Console.dll"));
        }
        let newest = root.join("shared/Microsoft.NETCore.App/10.0.11/System.Console.dll");
        assert_eq!(implementation_candidates(&reference, None), [newest, reference.clone()]);

        // The pack's own version wins.
        let exact = root.join("shared/Microsoft.NETCore.App/10.0.3/System.Console.dll");
        touch(&exact);
        assert_eq!(implementation_candidates(&reference, None), [exact, reference.clone()]);

        // An assembly the runtime does not have stays as it is.
        let other = root.join("packs/Microsoft.NETCore.App.Ref/10.0.3/ref/net10.0/Other.dll");
        touch(&other);
        assert_eq!(implementation_candidates(&other, None), std::slice::from_ref(&other));
    }

    #[test]
    fn a_pack_in_the_nuget_cache_uses_the_runtime_of_the_dotnet_host() {
        let dir = tempfile::tempdir().unwrap();
        let reference =
            dir.path().join("nuget/microsoft.aspnetcore.app.ref/8.0.24/ref/net8.0/Microsoft.AspNetCore.dll");
        touch(&reference);
        let dotnet = dir.path().join("dotnet");
        let runtime = dotnet.join("shared/Microsoft.AspNetCore.App/8.0.27/Microsoft.AspNetCore.dll");
        touch(&runtime);
        assert_eq!(implementation_candidates(&reference, None), std::slice::from_ref(&reference));
        assert_eq!(implementation_candidates(&reference, Some(&dotnet)), [runtime, reference.clone()]);
    }

    #[test]
    fn a_package_reference_assembly_resolves_to_its_lib_folder() {
        let dir = tempfile::tempdir().unwrap();
        let reference = dir.path().join("system.text.json/9.0.0/ref/net8.0/System.Text.Json.dll");
        touch(&reference);
        assert_eq!(implementation_candidates(&reference, None), std::slice::from_ref(&reference));
        let lib = dir.path().join("system.text.json/9.0.0/lib/net8.0/System.Text.Json.dll");
        touch(&lib);
        assert_eq!(implementation_candidates(&reference, None), [lib.clone(), reference]);
        // Already an implementation.
        assert_eq!(implementation_candidates(&lib, None), std::slice::from_ref(&lib));
    }

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(version_of("10.0.11"), Some((10, 0, 11)));
        assert_eq!(version_of("11.0.0-rc.1.25451.107"), Some((11, 0, 0)));
        assert_eq!(version_of("8.0"), Some((8, 0, 0)));
        assert_eq!(version_of("net8.0"), None);
    }
}
