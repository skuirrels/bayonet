using System.Reflection.Metadata;
using System.Reflection.PortableExecutable;
using ICSharpCode.Decompiler;
using ICSharpCode.Decompiler.Disassembler;
using ICSharpCode.Decompiler.Metadata;
// By another name: TraceEvent declares a `PEFile` namespace.
using AssemblyFile = ICSharpCode.Decompiler.Metadata.PEFile;

namespace Bayonet.Diagnostics;

/// <summary>
/// <c>Type</c> is a metadata name: <c>Namespace.Outer`1+Inner</c>. <c>Assemblies</c> are
/// tried in order (the implementation assembly first, then the one the code was compiled against).
/// </summary>
sealed record DisassembleParams(string[] Assemblies, string Type);

sealed record Disassembly(string Text, string Assembly, string Type, bool ReferenceAssembly);

/// <summary>IL of one type, as ILSpy's disassembler writes it.</summary>
static class Il
{
    const int MaxForwards = 4;

    public static Disassembly Disassemble(DisassembleParams p, CancellationToken ct)
    {
        if (p.Assemblies is not { Length: > 0 } || string.IsNullOrWhiteSpace(p.Type))
            throw new RpcException(RpcException.InvalidParams, "an assembly and a type are required");
        var names = p.Type.Split('+');
        string? unreadable = null;
        foreach (var assembly in p.Assemblies)
        {
            ct.ThrowIfCancellationRequested();
            Found? found;
            try
            {
                found = Find(assembly, names, MaxForwards);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException or BadImageFormatException or MetadataFileNotSupportedException)
            {
                unreadable ??= $"cannot read {assembly}: {e.Message}";
                continue;
            }
            if (found is null) continue;
            using var module = found.Module;
            return Write(module, found.Type, ct);
        }
        throw new RpcException(RpcException.Failed, unreadable ?? $"{p.Type} was not found in {p.Assemblies[0]}");
    }

    sealed record Found(AssemblyFile Module, TypeDefinitionHandle Type);

    /// <summary>The type in the assembly at <c>path</c>, or in the one it is forwarded to there.</summary>
    static Found? Find(string path, string[] names, int forwards)
    {
        var module = new AssemblyFile(path, PEStreamOptions.PrefetchEntireImage);
        var keep = false;
        try
        {
            var metadata = module.Metadata;
            var (ns, top) = SplitNamespace(names[0]);
            var type = Pick(metadata.TypeDefinitions.Where(h => !metadata.GetTypeDefinition(h).IsNested), top,
                h => metadata.GetTypeDefinition(h) is var d && metadata.StringComparer.Equals(d.Namespace, ns) ? metadata.GetString(d.Name) : null);
            if (type.IsNil)
            {
                if (forwards == 0) return null;
                var forwarded = Pick(metadata.ExportedTypes, top, h =>
                    metadata.GetExportedType(h) is { IsForwarder: true } e && metadata.StringComparer.Equals(e.Namespace, ns) ? metadata.GetString(e.Name) : null);
                if (forwarded.IsNil) return null;
                var target = metadata.GetAssemblyReference((AssemblyReferenceHandle)metadata.GetExportedType(forwarded).Implementation);
                var next = Path.Combine(Path.GetDirectoryName(path) ?? ".", metadata.GetString(target.Name) + ".dll");
                return File.Exists(next) ? Find(next, names, forwards - 1) : null;
            }
            foreach (var nested in names.Skip(1))
            {
                type = Pick(metadata.GetTypeDefinition(type).GetNestedTypes(), nested, h => metadata.GetString(metadata.GetTypeDefinition(h).Name));
                if (type.IsNil) return null;
            }
            keep = true;
            return new Found(module, type);
        }
        finally
        {
            if (!keep) module.Dispose();
        }
    }

    /// <summary>
    /// The candidate named <c>name</c>; failing that the only one with that name whatever
    /// its number of type parameters (<c>List</c> for <c>List`1</c>).
    /// </summary>
    static T Pick<T>(IEnumerable<T> candidates, string name, Func<T, string?> nameOf) where T : struct
    {
        var bare = Bare(name);
        var loose = new List<T>();
        foreach (var candidate in candidates)
        {
            if (nameOf(candidate) is not { } found) continue;
            if (found == name) return candidate;
            if (Bare(found) == bare) loose.Add(candidate);
        }
        return loose.Count == 1 ? loose[0] : default;
    }

    static string Bare(string name) => name.IndexOf('`') is >= 0 and var tick ? name[..tick] : name;

    static (string Namespace, string Name) SplitNamespace(string full)
    {
        var dot = full.LastIndexOf('.');
        return dot < 0 ? ("", full) : (full[..dot], full[(dot + 1)..]);
    }

    static Disassembly Write(AssemblyFile module, TypeDefinitionHandle type, CancellationToken ct)
    {
        var metadata = module.Metadata;
        var reference = metadata.GetAssemblyDefinition().GetCustomAttributes().Any(a => IsReferenceAssemblyAttribute(metadata, a));
        var name = type.GetFullTypeName(metadata).ReflectionName;
        var text = new StringWriter();
        text.WriteLine($"// IL of {name}");
        text.WriteLine($"// {module.FileName}");
        if (reference) text.WriteLine("// A reference assembly: it declares members without their bodies.");
        text.WriteLine($"// Disassembled with ICSharpCode.Decompiler {typeof(ReflectionDisassembler).Assembly.GetName().Version}");
        text.WriteLine();
        var disassembler = new ReflectionDisassembler(new PlainTextOutput(text), ct)
        {
            DetectControlStructure = true,
            ShowSequencePoints = false,
            // Attribute arguments as values rather than bytes; types of other assemblies are
            // looked up next to this one, and an argument that cannot be read stays as bytes.
            DecodeCustomAttributeBlobs = true,
            AssemblyResolver = new UniversalAssemblyResolver(module.FileName, false, module.DetectTargetFrameworkId()),
        };
        disassembler.DisassembleType(module, type);
        return new Disassembly(text.ToString(), module.FileName, name, reference);
    }

    static bool IsReferenceAssemblyAttribute(MetadataReader metadata, CustomAttributeHandle handle)
    {
        var constructor = metadata.GetCustomAttribute(handle).Constructor;
        EntityHandle declaring = constructor.Kind switch
        {
            HandleKind.MemberReference => metadata.GetMemberReference((MemberReferenceHandle)constructor).Parent,
            HandleKind.MethodDefinition => metadata.GetMethodDefinition((MethodDefinitionHandle)constructor).GetDeclaringType(),
            _ => default,
        };
        var (ns, name) = declaring.Kind switch
        {
            HandleKind.TypeReference => metadata.GetTypeReference((TypeReferenceHandle)declaring) is var r ? (r.Namespace, r.Name) : default,
            HandleKind.TypeDefinition => metadata.GetTypeDefinition((TypeDefinitionHandle)declaring) is var d ? (d.Namespace, d.Name) : default,
            _ => default,
        };
        return !name.IsNil
            && metadata.StringComparer.Equals(name, "ReferenceAssemblyAttribute")
            && metadata.StringComparer.Equals(ns, "System.Runtime.CompilerServices");
    }
}
