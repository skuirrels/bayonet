using Microsoft.Diagnostics.Symbols;
using Microsoft.Diagnostics.Tracing;
using Microsoft.Diagnostics.Tracing.Analysis;
using Microsoft.Diagnostics.Tracing.Etlx;
using Microsoft.Diagnostics.Tracing.Stacks;
using TraceLog = Microsoft.Diagnostics.Tracing.Etlx.TraceLog;

namespace Bayonet.Diagnostics;

record AnalyzeParams(string Path);

/// <summary>A distinct call stack, outermost frame first, with the time spent in it.</summary>
record Stack(int[] Frames, double Ms);

/// <summary>Time on one source line: <c>SelfMs</c> while it ran, <c>TotalMs</c> including what it called.</summary>
record LineTime(string File, int Line, double SelfMs, double TotalMs);

record Collection(double StartMs, double PauseMs, int Generation, string Reason, string Kind, double HeapBeforeMb, double HeapAfterMb);

record Analysis(
    double DurationMs,
    int Threads,
    string[] Frames,
    Stack[] Stacks,
    LineTime[] Lines,
    Collection[] Collections);

/// <summary>Reads a <c>.nettrace</c> into thread-time stacks (as dotnet-trace does), with source lines from portable PDBs.</summary>
static class Analyzer
{
    /// <summary>Line numbers portable PDBs use for hidden sequence points.</summary>
    const int HiddenLine = 0xFEEFEE;

    public static Analysis Analyze(AnalyzeParams p, CancellationToken ct)
    {
        if (!File.Exists(p.Path)) throw new RpcException(RpcException.InvalidParams, $"no such trace: {p.Path}");
        var scratch = Directory.CreateTempSubdirectory("bayonet-trace-");
        try
        {
            var etlx = TraceLog.CreateFromEventPipeDataFile(p.Path, Path.Combine(scratch.FullName, "trace.etlx"));
            ct.ThrowIfCancellationRequested();
            using var log = new TraceLog(etlx);
            using var symbols = new SymbolReader(TextWriter.Null, "") { SecurityCheck = _ => true };
            var analysis = Samples(log, symbols, ct);
            return analysis with { Collections = Collections(log) };
        }
        finally
        {
            try { scratch.Delete(recursive: true); } catch (IOException) { }
        }
    }

    static Analysis Samples(TraceLog log, SymbolReader symbols, CancellationToken ct)
    {
        var source = new MutableTraceEventStackSource(log) { OnlyManagedCodeStacks = true };
        new SampleProfilerThreadTimeComputer(log, symbols).GenerateThreadTimeStacks(source);

        var frameIds = new Dictionary<StackSourceFrameIndex, int>();
        var frames = new List<string>();
        var lineOf = new Dictionary<StackSourceFrameIndex, (string File, int Line)?>();
        var stacks = new Dictionary<string, (int[] Frames, double Ms)>();
        var lines = new Dictionary<(string, int), (double Self, double Total)>();
        var threads = new HashSet<string>();
        var walk = new List<StackSourceFrameIndex>();
        var seen = new HashSet<(string, int)>();
        var count = 0;

        source.ForEach(sample =>
        {
            if ((++count & 0xFFF) == 0) ct.ThrowIfCancellationRequested();
            walk.Clear();
            for (var s = sample.StackIndex; s != StackSourceCallStackIndex.Invalid; s = source.GetCallerIndex(s))
                walk.Add(source.GetFrameIndex(s));
            walk.Reverse();

            var ids = new int[walk.Count];
            for (var i = 0; i < walk.Count; i++)
            {
                if (!frameIds.TryGetValue(walk[i], out var id))
                {
                    id = frames.Count;
                    frameIds[walk[i]] = id;
                    var name = source.GetFrameName(walk[i], false);
                    frames.Add(name);
                    if (name.StartsWith("Thread (", StringComparison.Ordinal)) threads.Add(name);
                }
                ids[i] = id;
            }
            var key = string.Join(',', ids);
            stacks[key] = stacks.TryGetValue(key, out var existing) ? (existing.Frames, existing.Ms + sample.Metric) : (ids, sample.Metric);

            // The innermost frame with a source line is where the time was spent; every
            // line above it on the stack is a call that led there.
            seen.Clear();
            var leaf = true;
            for (var i = walk.Count - 1; i >= 0; i--)
            {
                if (SourceLine(log, source, symbols, walk[i], lineOf) is not { } at) continue;
                var k = (at.File, at.Line);
                var (self, total) = lines.GetValueOrDefault(k);
                if (seen.Add(k)) total += sample.Metric;
                if (leaf) self += sample.Metric;
                leaf = false;
                lines[k] = (self, total);
            }
        });

        return new Analysis(
            log.SessionDuration.TotalMilliseconds,
            threads.Count,
            [.. frames],
            [.. stacks.Values.Select(s => new Stack(s.Frames, Math.Round(s.Ms, 3)))],
            [.. lines.Select(l => new LineTime(l.Key.Item1, l.Key.Item2, Math.Round(l.Value.Self, 3), Math.Round(l.Value.Total, 3)))],
            []);
    }

    static (string File, int Line)? SourceLine(
        TraceLog log,
        MutableTraceEventStackSource source,
        SymbolReader symbols,
        StackSourceFrameIndex frame,
        Dictionary<StackSourceFrameIndex, (string, int)?> cache)
    {
        if (cache.TryGetValue(frame, out var known)) return known;
        (string, int)? found = null;
        var address = source.GetFrameCodeAddress(frame);
        if (address != CodeAddressIndex.Invalid && log.CodeAddresses.MethodIndex(address) != MethodIndex.Invalid)
        {
            try
            {
                var at = log.CodeAddresses.GetSourceLine(symbols, address);
                if (at is { LineNumber: > 0 and < HiddenLine, SourceFile.BuildTimeFilePath: { Length: > 0 } file })
                    found = (file, at.LineNumber);
            }
            catch (Exception e) when (e is IOException or InvalidOperationException or BadImageFormatException or UnauthorizedAccessException)
            {
            }
        }
        cache[frame] = found;
        return found;
    }

    static Collection[] Collections(TraceLog log)
    {
        var events = log.Events.GetSource();
        events.NeedLoadedDotNetRuntimes();
        events.Process();
        var list = new List<Collection>();
        foreach (var process in events.Processes())
        {
            if (process.LoadedDotNetRuntime() is not { } runtime) continue;
            foreach (var gc in runtime.GC.GCs)
            {
                list.Add(new Collection(
                    Math.Round(gc.StartRelativeMSec, 3),
                    Math.Round(gc.PauseDurationMSec, 3),
                    gc.Generation,
                    gc.Reason.ToString(),
                    gc.Type.ToString(),
                    Math.Round(gc.HeapSizeBeforeMB, 3),
                    Math.Round(gc.HeapSizeAfterMB, 3)));
            }
        }
        return [.. list.OrderBy(c => c.StartMs)];
    }
}
