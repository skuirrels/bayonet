using System.Reflection;
using System.Runtime.Loader;

namespace Bayonet.Diagnostics;

/// <param name="Path">A <c>.gcdump</c> file.</param>
/// <param name="Tool">The installed <c>dotnet-gcdump.dll</c>, whose reader loads the file.</param>
/// <param name="Types">How many types to report, largest retained size first.</param>
/// <param name="Paths">How many of those get their paths from the roots.</param>
sealed record RetentionParams(string Path, string Tool, int? Types, int? Paths);

/// <summary>
/// What keeps the objects of a heap snapshot alive: the retained size of each type (what
/// the collector would free with its objects, from the dominator tree) and the shortest
/// reference paths from the roots to its objects.
/// </summary>
static class Heap
{
    const int MaxFrames = 16;
    const int MaxSampledObjects = 2000;
    const int MaxPathLength = 10_000;
    const int PathsPerType = 6;

    public static object Retention(RetentionParams p, CancellationToken ct)
    {
        if (!File.Exists(p.Path)) throw new RpcException(RpcException.InvalidParams, $"{p.Path} does not exist");
        if (!File.Exists(p.Tool)) throw new RpcException(RpcException.InvalidParams, $"{p.Tool} does not exist");
        HeapGraph graph;
        try
        {
            graph = HeapGraph.Load(p.Tool, p.Path, ct);
        }
        catch (TargetInvocationException e) when (e.InnerException is { } inner)
        {
            throw new RpcException(RpcException.Failed, $"cannot read {p.Path}: {inner.Message}");
        }
        var retained = graph.Dominate(ct);
        var byType = graph.RetainedByType(retained, ct);
        var parents = graph.ShortestPathParents();

        // Size buckets of one type (`System.Byte[] (Bytes > 10K)`) are one type, as in the report.
        var names = graph.TypeNames.Select(Unbucketed).ToArray();
        var merged = new Dictionary<string, (long Retained, List<int> Types)>();
        for (var t = 0; t < names.Length; t++)
        {
            if (byType[t] == 0) continue;
            var entry = merged.TryGetValue(names[t], out var found) ? found : (0, []);
            entry.Types.Add(t);
            merged[names[t]] = (entry.Retained + byType[t], entry.Types);
        }
        var ordered = merged.OrderByDescending(e => e.Value.Retained).ThenBy(e => e.Key, StringComparer.Ordinal)
            .Take(Math.Clamp(p.Types ?? 2000, 1, 20_000)).ToList();
        var withPaths = Math.Clamp(p.Paths ?? 200, 0, ordered.Count);
        var objects = new List<int>[names.Length];
        var wanted = ordered.Take(withPaths).SelectMany(e => e.Value.Types).ToHashSet();
        foreach (var t in wanted) objects[t] = [];
        for (var node = 0; node < graph.Count; node++)
        {
            if (parents[node] != HeapGraph.Unreachable && objects[graph.Type[node]] is { } list) list.Add(node);
        }
        var types = ordered.Select((e, i) => new
        {
            name = e.Key,
            retainedBytes = e.Value.Retained,
            paths = i < withPaths ? PathsTo(graph, parents, retained, e.Value.Types.SelectMany(t => objects[t]).ToList(), names, ct) : [],
        }).ToList();
        return new { totalBytes = retained[graph.Root], objects = graph.Reachable, types };
    }

    static string Unbucketed(string name)
    {
        var at = name.LastIndexOf(" (Bytes > ", StringComparison.Ordinal);
        return at > 0 && name.EndsWith(')') ? name[..at] : name;
    }

    /// <summary>The commonest type chains from a root to <paramref name="objects"/>, root first.</summary>
    static List<object> PathsTo(HeapGraph graph, int[] parents, long[] retained, List<int> objects, string[] names, CancellationToken ct)
    {
        // A sample of evenly spaced objects stands for all of them.
        var step = Math.Max(1, objects.Count / MaxSampledObjects);
        var groups = new Dictionary<string, (long Count, long Bytes, List<int> Types)>();
        var chain = new List<int>();
        for (var i = 0; i < objects.Count; i += step)
        {
            ct.ThrowIfCancellationRequested();
            chain.Clear();
            var node = objects[i];
            for (var parent = parents[node]; parent >= 0 && chain.Count < MaxPathLength; parent = parents[parent])
            {
                // A run of one type (a linked list, a tree) is one frame.
                var type = graph.Type[parent];
                if (chain.Count == 0 || chain[^1] != type) chain.Add(type);
            }
            var key = string.Join(',', chain);
            var group = groups.TryGetValue(key, out var found) ? found : (0, 0, [.. chain]);
            groups[key] = (group.Count + step, group.Bytes + retained[node] * step, group.Types);
        }
        return groups.Values.OrderByDescending(g => g.Bytes).ThenByDescending(g => g.Count).Take(PathsPerType).Select(g =>
        {
            var frames = g.Types.AsEnumerable().Reverse().Select(t => names[t]).ToList();
            if (frames.Count > MaxFrames)
            {
                frames = [.. frames.Take(6), "…", .. frames.Skip(frames.Count - (MaxFrames - 7))];
            }
            return (object)new { count = Math.Min(g.Count, objects.Count), retainedBytes = g.Bytes, frames };
        }).ToList();
    }
}

/// <summary>dotnet-gcdump's assemblies, apart from this helper's own TraceEvent.</summary>
sealed class GcDumpTool(string dll) : AssemblyLoadContext("dotnet-gcdump")
{
    readonly AssemblyDependencyResolver resolver = new(dll);

    protected override Assembly? Load(AssemblyName name) =>
        resolver.ResolveAssemblyToPath(name) is { } path ? LoadFromAssemblyPath(path) : null;
}

/// <summary>The objects of a heap snapshot and the references between them, as flat arrays.</summary>
sealed class HeapGraph
{
    public const int Unreachable = -2;

    public int Root { get; private init; }
    public int Count => Size.Length;
    /// <summary>Objects reachable from the root; set by <see cref="Dominate"/>.</summary>
    public int Reachable { get; private set; }
    public required int[] Size { get; init; }
    public required int[] Type { get; init; }
    /// <summary>The references of object <c>i</c> are <c>Edges[Start[i]..Start[i + 1]]</c>.</summary>
    public required int[] Start { get; init; }
    public required int[] Edges { get; init; }
    public required string[] TypeNames { get; init; }

    int[] order = [];
    int[] dominator = [];

    /// <summary>
    /// Reads a <c>.gcdump</c> with the reader of the installed dotnet-gcdump: the format is
    /// that tool's own, and its graph classes are in no package.
    /// </summary>
    public static HeapGraph Load(string tool, string path, CancellationToken ct)
    {
        var assembly = new GcDumpTool(tool).LoadFromAssemblyPath(tool);
        Type Find(string name) => assembly.GetType(name)
            ?? throw new RpcException(RpcException.Failed, $"this dotnet-gcdump has no {name}; update it");
        var nodeIndex = Find("Graphs.NodeIndex");
        var typeIndex = Find("Graphs.NodeTypeIndex");
        dynamic dump = Activator.CreateInstance(Find("GCHeapDump"), path)!;
        dynamic graph = dump.MemoryGraph;
        var count = (int)graph.NodeIndexLimit;
        var size = new int[count];
        var type = new int[count];
        var start = new int[count + 1];
        var edges = new List<int>((int)graph.TotalNumberOfReferences);
        dynamic storage = graph.AllocNodeStorage();
        for (var i = 0; i < count; i++)
        {
            if ((i & 0xfff) == 0) ct.ThrowIfCancellationRequested();
            dynamic node = graph.GetNode((dynamic)Enum.ToObject(nodeIndex, i), storage);
            size[i] = (int)node.Size;
            type[i] = (int)node.TypeIndex;
            start[i] = edges.Count;
            for (var child = (int)node.GetFirstChildIndex(); child != -1; child = (int)node.GetNextChildIndex())
            {
                if ((uint)child < (uint)count) edges.Add(child);
            }
        }
        start[count] = edges.Count;
        var typeCount = (int)graph.NodeTypeIndexLimit;
        var names = new string[typeCount];
        dynamic typeStorage = graph.AllocTypeNodeStorage();
        for (var t = 0; t < typeCount; t++)
        {
            dynamic nodeType = graph.GetType((dynamic)Enum.ToObject(typeIndex, t), typeStorage);
            names[t] = (string?)nodeType.Name ?? $"type {t}";
        }
        var root = (int)graph.RootIndex;
        if ((uint)root >= (uint)count) throw new RpcException(RpcException.Failed, $"{path} has no objects");
        return new HeapGraph { Root = root, Size = size, Type = type, Start = start, Edges = [.. edges], TypeNames = names };
    }

    /// <summary>
    /// The retained size of every object: its own and that of all it dominates (every path
    /// from the root to them goes through it). Cooper, Harvey and Kennedy's iterative algorithm.
    /// </summary>
    public long[] Dominate(CancellationToken ct)
    {
        var n = Count;
        // Depth-first from the root, objects numbered in reverse postorder.
        var post = new int[n];
        var reached = 0;
        var visited = new bool[n];
        var stack = new int[n];
        var cursor = new int[n];
        var depth = 0;
        stack[depth++] = Root;
        visited[Root] = true;
        cursor[Root] = Start[Root];
        while (depth > 0)
        {
            var v = stack[depth - 1];
            if (cursor[v] < Start[v + 1])
            {
                var w = Edges[cursor[v]++];
                if (visited[w]) continue;
                visited[w] = true;
                cursor[w] = Start[w];
                stack[depth++] = w;
            }
            else
            {
                post[reached++] = v;
                depth--;
            }
        }
        Reachable = reached;
        order = new int[reached];
        var number = new int[n];
        Array.Fill(number, -1);
        for (var i = 0; i < reached; i++)
        {
            order[i] = post[reached - 1 - i];
            number[order[i]] = i;
        }

        // Who refers to each reachable object.
        var incoming = new int[n + 1];
        for (var i = 0; i < reached; i++)
        {
            var v = order[i];
            for (var e = Start[v]; e < Start[v + 1]; e++) incoming[Edges[e] + 1]++;
        }
        for (var i = 0; i < n; i++) incoming[i + 1] += incoming[i];
        var referrers = new int[incoming[n]];
        var fill = (int[])incoming.Clone();
        for (var i = 0; i < reached; i++)
        {
            var v = order[i];
            for (var e = Start[v]; e < Start[v + 1]; e++) referrers[fill[Edges[e]]++] = v;
        }

        dominator = new int[n];
        Array.Fill(dominator, -1);
        dominator[Root] = Root;
        for (var changed = true; changed;)
        {
            ct.ThrowIfCancellationRequested();
            changed = false;
            for (var i = 1; i < reached; i++)
            {
                var b = order[i];
                var best = -1;
                for (var e = incoming[b]; e < incoming[b + 1]; e++)
                {
                    var p = referrers[e];
                    if (dominator[p] == -1) continue;
                    if (best == -1)
                    {
                        best = p;
                        continue;
                    }
                    var other = p;
                    while (other != best)
                    {
                        while (number[other] > number[best]) other = dominator[other];
                        while (number[best] > number[other]) best = dominator[best];
                    }
                }
                if (best != dominator[b])
                {
                    dominator[b] = best;
                    changed = true;
                }
            }
        }

        var retained = new long[n];
        for (var i = 0; i < reached; i++) retained[order[i]] = Size[order[i]];
        // A dominator comes before what it dominates: add up from the far end.
        for (var i = reached - 1; i > 0; i--) retained[dominator[order[i]]] += retained[order[i]];
        return retained;
    }

    /// <summary>
    /// The retained size per type: of its objects that no other object of the type
    /// dominates, so nothing counts twice. After <see cref="Dominate"/>.
    /// </summary>
    public long[] RetainedByType(long[] retained, CancellationToken ct)
    {
        var total = new long[TypeNames.Length];
        // What each object dominates directly.
        var first = new int[Count + 1];
        for (var i = 1; i < Reachable; i++) first[dominator[order[i]] + 1]++;
        for (var i = 0; i < Count; i++) first[i + 1] += first[i];
        var dominated = new int[first[Count]];
        var fill = (int[])first.Clone();
        for (var i = 1; i < Reachable; i++) dominated[fill[dominator[order[i]]]++] = order[i];

        // Down the dominator tree, counting the objects of each type on the way to here.
        var open = new int[TypeNames.Length];
        var stack = new int[Reachable];
        var cursor = new int[Count];
        var depth = 0;
        void Enter(int v)
        {
            if (open[Type[v]]++ == 0) total[Type[v]] += retained[v];
            cursor[v] = first[v];
            stack[depth++] = v;
        }
        Enter(Root);
        for (var steps = 0; depth > 0; steps++)
        {
            if ((steps & 0xffff) == 0) ct.ThrowIfCancellationRequested();
            var v = stack[depth - 1];
            if (cursor[v] < first[v + 1])
            {
                Enter(dominated[cursor[v]++]);
            }
            else
            {
                open[Type[v]]--;
                depth--;
            }
        }
        return total;
    }

    /// <summary>Each object's referrer on a shortest path from the root (−1 for the root).</summary>
    public int[] ShortestPathParents()
    {
        var parents = new int[Count];
        Array.Fill(parents, Unreachable);
        var queue = new int[Count];
        var (head, tail) = (0, 0);
        queue[tail++] = Root;
        parents[Root] = -1;
        while (head < tail)
        {
            var v = queue[head++];
            for (var e = Start[v]; e < Start[v + 1]; e++)
            {
                var w = Edges[e];
                if (parents[w] != Unreachable) continue;
                parents[w] = v;
                queue[tail++] = w;
            }
        }
        return parents;
    }
}
