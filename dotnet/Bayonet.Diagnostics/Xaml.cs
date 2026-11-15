using System.Collections.Immutable;
using System.Reflection;
using System.Reflection.Metadata;
using System.Reflection.PortableExecutable;

namespace Bayonet.Diagnostics;

/// <summary>
/// <c>Assemblies</c>: what the project compiles against. <c>Own</c>: the project's built
/// assembly, whose types XAML names through <c>clr-namespace:</c> or <c>using:</c>.
/// </summary>
sealed record XamlTypesParams(string[] Assemblies, string[]? Own);

sealed record XmlnsMapping(string Xmlns, string ClrNamespace);

sealed record XamlProperty(string Name, string Type, string[]? Values);

sealed record XamlType(
    string Namespace,
    string Name,
    string? Base,
    bool Abstract,
    XamlProperty[] Properties,
    string[] Events,
    XamlProperty[] Attached);

sealed record XamlTypes(XmlnsMapping[] Namespaces, XamlType[] Types);

/// <summary>
/// What XAML can name (CI-16): the types of the UI framework assemblies (those that map
/// an XML namespace with <c>XmlnsDefinitionAttribute</c>: WPF, Avalonia, MAUI, WinUI) and
/// of the project, with their settable properties, events and attached properties. Read
/// from metadata only; nothing is loaded.
/// </summary>
static class Xaml
{
    public static XamlTypes Types(XamlTypesParams p, CancellationToken ct)
    {
        var mappings = new List<XmlnsMapping>();
        var read = new List<(MetadataReader Reader, bool Own)>();
        var files = new List<PEReader>();
        try
        {
            var own = new HashSet<string>(p.Own ?? [], StringComparer.OrdinalIgnoreCase);
            foreach (var path in p.Assemblies.Concat(own).Distinct(StringComparer.OrdinalIgnoreCase))
            {
                ct.ThrowIfCancellationRequested();
                PEReader pe;
                try
                {
                    pe = new PEReader(File.OpenRead(path));
                    if (!pe.HasMetadata) { pe.Dispose(); continue; }
                }
                catch (Exception e) when (e is IOException or UnauthorizedAccessException or BadImageFormatException)
                {
                    continue;
                }
                files.Add(pe);
                var reader = pe.GetMetadataReader();
                mappings.AddRange(XmlnsDefinitions(reader));
                read.Add((reader, own.Contains(path)));
            }
            var known = new Known();
            foreach (var (reader, _) in read)
            {
                CollectEnums(reader, known.Enums);
                CollectShapes(reader, known.Shapes);
            }
            // A mapped namespace may hold types of assemblies that map nothing themselves
            // (MAUI maps namespaces of other assemblies; libraries add to a framework's).
            var mappedNamespaces = mappings.Select(m => m.ClrNamespace).ToHashSet();
            var types = new List<XamlType>();
            foreach (var (reader, isOwn) in read)
            {
                ct.ThrowIfCancellationRequested();
                types.AddRange(TypesOf(reader, isOwn ? null : mappedNamespaces, known));
            }
            return new XamlTypes([.. mappings.Distinct()], [.. types]);
        }
        finally
        {
            foreach (var f in files) f.Dispose();
        }
    }

    static List<XmlnsMapping> XmlnsDefinitions(MetadataReader reader)
    {
        var out_ = new List<XmlnsMapping>();
        foreach (var handle in reader.GetAssemblyDefinition().GetCustomAttributes())
        {
            var attribute = reader.GetCustomAttribute(handle);
            if (AttributeName(reader, attribute) != "XmlnsDefinitionAttribute") continue;
            string? xmlns, clr;
            try
            {
                var blob = reader.GetBlobReader(attribute.Value);
                if (blob.Length < 2 || blob.ReadUInt16() != 1) continue;
                xmlns = blob.ReadSerializedString();
                clr = blob.ReadSerializedString();
            }
            catch (BadImageFormatException)
            {
                continue;
            }
            if (!string.IsNullOrEmpty(xmlns) && !string.IsNullOrEmpty(clr)) out_.Add(new XmlnsMapping(xmlns, clr));
        }
        return out_;
    }

    static string? AttributeName(MetadataReader reader, CustomAttribute attribute)
    {
        switch (attribute.Constructor.Kind)
        {
            case HandleKind.MemberReference:
                var parent = reader.GetMemberReference((MemberReferenceHandle)attribute.Constructor).Parent;
                return parent.Kind switch
                {
                    HandleKind.TypeReference => reader.GetString(reader.GetTypeReference((TypeReferenceHandle)parent).Name),
                    HandleKind.TypeDefinition => reader.GetString(reader.GetTypeDefinition((TypeDefinitionHandle)parent).Name),
                    _ => null,
                };
            case HandleKind.MethodDefinition:
                var owner = reader.GetMethodDefinition((MethodDefinitionHandle)attribute.Constructor).GetDeclaringType();
                return reader.GetString(reader.GetTypeDefinition(owner).Name);
            default:
                return null;
        }
    }

    static bool IsPublic(TypeDefinition t) =>
        (t.Attributes & TypeAttributes.VisibilityMask) == TypeAttributes.Public;

    /// <summary>A type's base and interfaces, to tell collections from other types.</summary>
    sealed record Shape(string? Base, string[] Interfaces);

    /// <summary>What every read assembly declares, by full type name: enum values and type shapes.</summary>
    sealed class Known
    {
        public readonly Dictionary<string, string[]> Enums = [];
        public readonly Dictionary<string, Shape> Shapes = [];
    }

    static string Bare(string type)
    {
        var open = type.IndexOf('<');
        return open >= 0 ? type[..open] : type;
    }

    static void CollectShapes(MetadataReader reader, Dictionary<string, Shape> shapes)
    {
        foreach (var handle in reader.TypeDefinitions)
        {
            try
            {
                var t = reader.GetTypeDefinition(handle);
                if (!IsPublic(t)) continue;
                var name = reader.GetString(t.Name);
                var tick = name.IndexOf('`');
                if (tick >= 0) name = name[..tick];
                var interfaces = t.GetInterfaceImplementations()
                    .Select(i => TypeNames.BaseName(reader, reader.GetInterfaceImplementation(i).Interface))
                    .OfType<string>()
                    .Select(Bare)
                    .ToArray();
                shapes.TryAdd($"{reader.GetString(t.Namespace)}.{name}", new Shape(TypeNames.BaseName(reader, t.BaseType) is { } b ? Bare(b) : null, interfaces));
            }
            catch (BadImageFormatException)
            {
            }
        }
    }

    /// <summary>Whether XAML can fill a property of <c>type</c> in place, as a list or dictionary.</summary>
    static bool IsCollection(string type, Known known)
    {
        var name = Bare(type);
        for (var depth = 0; depth < 16 && name is not null; depth++)
        {
            if (name.EndsWith("Collection", StringComparison.Ordinal)) return true;
            if (name is "System.Collections.IList" or "System.Collections.IDictionary" or "System.Collections.Generic.IList" or "System.Collections.Generic.ICollection" or "System.Collections.Generic.IDictionary") return true;
            if (!known.Shapes.TryGetValue(name, out var shape)) return false;
            if (shape.Interfaces.Any(i => i.Contains("IList", StringComparison.Ordinal) || i.Contains("ICollection", StringComparison.Ordinal) || i.Contains("IDictionary", StringComparison.Ordinal))) return true;
            name = shape.Base;
        }
        return false;
    }

    static void CollectEnums(MetadataReader reader, Dictionary<string, string[]> enums)
    {
        foreach (var handle in reader.TypeDefinitions)
        {
            var t = reader.GetTypeDefinition(handle);
            if (!IsPublic(t) || TypeNames.BaseName(reader, t.BaseType) != "System.Enum") continue;
            var values = t.GetFields()
                .Select(reader.GetFieldDefinition)
                .Where(f => (f.Attributes & FieldAttributes.Static) != 0)
                .Select(f => reader.GetString(f.Name))
                .ToArray();
            enums[$"{reader.GetString(t.Namespace)}.{reader.GetString(t.Name)}"] = values;
        }
    }

    static IEnumerable<XamlType> TypesOf(MetadataReader reader, HashSet<string>? namespaces, Known known)
    {
        var names = new TypeNames(reader);
        foreach (var handle in reader.TypeDefinitions)
        {
            XamlType? type;
            try
            {
                type = TypeOf(reader, handle, names, namespaces, known);
            }
            catch (BadImageFormatException)
            {
                // A signature this reader cannot decode: leave the type out rather than all of them.
                continue;
            }
            if (type is not null) yield return type;
        }
    }

    static XamlType? TypeOf(MetadataReader reader, TypeDefinitionHandle handle, TypeNames names, HashSet<string>? namespaces, Known known)
    {
        {
            var t = reader.GetTypeDefinition(handle);
            if (!IsPublic(t) || (t.Attributes & TypeAttributes.Interface) != 0) return null;
            var ns = reader.GetString(t.Namespace);
            if (namespaces is not null && !namespaces.Contains(ns)) return null;
            var name = reader.GetString(t.Name);
            if (name.Contains('`') || name.StartsWith('<')) return null;
            var baseName = TypeNames.BaseName(reader, t.BaseType);
            if (baseName is "System.Enum" or "System.ValueType" or "System.MulticastDelegate" or "System.Attribute") return null;
            var sealedAbstract = (t.Attributes & (TypeAttributes.Abstract | TypeAttributes.Sealed)) == (TypeAttributes.Abstract | TypeAttributes.Sealed);

            var properties = new List<XamlProperty>();
            var attached = new List<XamlProperty>();
            var events = new List<string>();
            foreach (var ph in t.GetProperties())
            {
                var prop = reader.GetPropertyDefinition(ph);
                var accessors = prop.GetAccessors();
                if (accessors.Getter.IsNil && accessors.Setter.IsNil) continue;
                var anyAccessor = accessors.Setter.IsNil ? accessors.Getter : accessors.Setter;
                var accessor = reader.GetMethodDefinition(anyAccessor);
                if ((accessor.Attributes & MethodAttributes.Static) != 0) continue;
                var type = prop.DecodeSignature(names, null).ReturnType;
                var settable = !accessors.Setter.IsNil && IsPublicMethod(reader, accessors.Setter);
                // Collections are filled in place (`<Grid.RowDefinitions>`), so a get-only one counts.
                var collection = !settable && !accessors.Getter.IsNil && IsPublicMethod(reader, accessors.Getter)
                    && IsCollection(type, known);
                if (!settable && !collection) continue;
                properties.Add(new XamlProperty(reader.GetString(prop.Name), type, ValuesOf(type, known.Enums)));
            }
            foreach (var eh in t.GetEvents())
            {
                var ev = reader.GetEventDefinition(eh);
                var adder = ev.GetAccessors().Adder;
                if (adder.IsNil || !IsPublicMethod(reader, adder)) continue;
                if ((reader.GetMethodDefinition(adder).Attributes & MethodAttributes.Static) != 0) continue;
                events.Add(reader.GetString(ev.Name));
            }
            foreach (var mh in t.GetMethods())
            {
                var m = reader.GetMethodDefinition(mh);
                if ((m.Attributes & (MethodAttributes.Static | MethodAttributes.MemberAccessMask)) != (MethodAttributes.Static | MethodAttributes.Public)) continue;
                var method = reader.GetString(m.Name);
                if (method.Length <= 3 || !method.StartsWith("Set", StringComparison.Ordinal)) continue;
                var signature = m.DecodeSignature(names, null);
                if (signature.ParameterTypes.Length != 2 || signature.ReturnType != "System.Void") continue;
                var type = signature.ParameterTypes[1];
                attached.Add(new XamlProperty(method[3..], type, ValuesOf(type, known.Enums)));
            }
            if (sealedAbstract && attached.Count == 0) return null;
            return new XamlType(
                ns,
                name,
                baseName,
                (t.Attributes & TypeAttributes.Abstract) != 0,
                [.. properties.DistinctBy(x => x.Name)],
                [.. events.Distinct()],
                [.. attached.DistinctBy(x => x.Name)]);
        }
    }

    static bool IsPublicMethod(MetadataReader reader, MethodDefinitionHandle h) =>
        (reader.GetMethodDefinition(h).Attributes & MethodAttributes.MemberAccessMask) == MethodAttributes.Public;

    static string[]? ValuesOf(string type, Dictionary<string, string[]> enums)
    {
        var bare = type.EndsWith('?') ? type[..^1] : type;
        if (bare == "System.Boolean") return ["True", "False"];
        return enums.TryGetValue(bare, out var values) ? values : null;
    }

    /// <summary>Type names from signatures: <c>System.Double</c>, <c>Avalonia.Controls.Controls</c>.</summary>
    sealed class TypeNames(MetadataReader reader) : ISignatureTypeProvider<string, object?>
    {
        public static string? BaseName(MetadataReader reader, EntityHandle handle) => handle.IsNil ? null : handle.Kind switch
        {
            HandleKind.TypeReference => Full(reader, (TypeReferenceHandle)handle),
            HandleKind.TypeDefinition => Full(reader, (TypeDefinitionHandle)handle),
            HandleKind.TypeSpecification => Generic(reader, (TypeSpecificationHandle)handle),
            _ => null,
        };

        static string Full(MetadataReader reader, TypeReferenceHandle h)
        {
            var r = reader.GetTypeReference(h);
            var name = reader.GetString(r.Name);
            if (r.ResolutionScope.Kind == HandleKind.TypeReference)
                return $"{Full(reader, (TypeReferenceHandle)r.ResolutionScope)}+{name}";
            var ns = reader.GetString(r.Namespace);
            return ns.Length == 0 ? name : $"{ns}.{name}";
        }

        static string Full(MetadataReader reader, TypeDefinitionHandle h)
        {
            var d = reader.GetTypeDefinition(h);
            var name = reader.GetString(d.Name);
            var outer = d.GetDeclaringType();
            if (!outer.IsNil) return $"{Full(reader, outer)}+{name}";
            var ns = reader.GetString(d.Namespace);
            return ns.Length == 0 ? name : $"{ns}.{name}";
        }

        /// <summary>The generic type of a base such as <c>ItemsControl&lt;T&gt;</c>, without its arguments.</summary>
        static string? Generic(MetadataReader reader, TypeSpecificationHandle h)
        {
            var name = reader.GetTypeSpecification(h).DecodeSignature(new TypeNames(reader), null);
            var open = name.IndexOf('<');
            return open >= 0 ? name[..open] : name;
        }

        public string GetPrimitiveType(PrimitiveTypeCode typeCode) => typeCode switch
        {
            PrimitiveTypeCode.Void => "System.Void",
            PrimitiveTypeCode.Boolean => "System.Boolean",
            PrimitiveTypeCode.Char => "System.Char",
            PrimitiveTypeCode.SByte => "System.SByte",
            PrimitiveTypeCode.Byte => "System.Byte",
            PrimitiveTypeCode.Int16 => "System.Int16",
            PrimitiveTypeCode.UInt16 => "System.UInt16",
            PrimitiveTypeCode.Int32 => "System.Int32",
            PrimitiveTypeCode.UInt32 => "System.UInt32",
            PrimitiveTypeCode.Int64 => "System.Int64",
            PrimitiveTypeCode.UInt64 => "System.UInt64",
            PrimitiveTypeCode.Single => "System.Single",
            PrimitiveTypeCode.Double => "System.Double",
            PrimitiveTypeCode.String => "System.String",
            PrimitiveTypeCode.Object => "System.Object",
            PrimitiveTypeCode.IntPtr => "System.IntPtr",
            PrimitiveTypeCode.UIntPtr => "System.UIntPtr",
            _ => typeCode.ToString(),
        };

        public string GetTypeFromDefinition(MetadataReader r, TypeDefinitionHandle handle, byte rawTypeKind) => Full(r, handle);
        public string GetTypeFromReference(MetadataReader r, TypeReferenceHandle handle, byte rawTypeKind) => Full(r, handle);
        public string GetTypeFromSpecification(MetadataReader r, object? context, TypeSpecificationHandle handle, byte rawTypeKind) =>
            r.GetTypeSpecification(handle).DecodeSignature(this, context);
        public string GetSZArrayType(string elementType) => elementType + "[]";
        public string GetArrayType(string elementType, ArrayShape shape) => elementType + "[" + new string(',', shape.Rank - 1) + "]";
        public string GetByReferenceType(string elementType) => elementType;
        public string GetPointerType(string elementType) => elementType + "*";
        public string GetPinnedType(string elementType) => elementType;
        public string GetFunctionPointerType(MethodSignature<string> signature) => "System.IntPtr";
        public string GetGenericMethodParameter(object? context, int index) => $"!!{index}";
        public string GetGenericTypeParameter(object? context, int index) => $"!{index}";
        public string GetModifiedType(string modifier, string unmodifiedType, bool isRequired) => unmodifiedType;

        public string GetGenericInstantiation(string genericType, ImmutableArray<string> typeArguments)
        {
            var tick = genericType.IndexOf('`');
            var bare = tick >= 0 ? genericType[..tick] : genericType;
            return bare == "System.Nullable" && typeArguments.Length == 1
                ? typeArguments[0] + "?"
                : $"{bare}<{string.Join(",", typeArguments)}>";
        }

        // Only the type names are wanted; `reader` keeps the primary constructor's parameter used.
        public MetadataReader Reader => reader;
    }
}
