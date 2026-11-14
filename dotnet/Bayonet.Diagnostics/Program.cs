// Bayonet's .NET diagnostics helper: records EventPipe traces of a running or newly
// started .NET process and reads them into pre-aggregated results for the IDE
// (call stacks with source lines, garbage collections). It also disassembles a type of
// an assembly to IL, and works out what retains the objects of a heap snapshot,
// and lists the types XAML can name. Speaks
// JSON-RPC on stdio.
using System.Text.Json;
using Bayonet.Diagnostics;

var server = new RpcServer(Console.OpenStandardInput(), Console.OpenStandardOutput());

server.On("initialize", (_, _) => Task.FromResult<object?>(new
{
    version = Protocol.Version,
    runtime = Environment.Version.ToString(),
}));
server.On("trace/collect", async (p, ct) => await Recorder.Attach(Params<AttachParams>(p), ct));
server.On("trace/launch", async (p, ct) => await Recorder.Launch(Params<LaunchParams>(p), ct));
server.On("trace/analyze", async (p, ct) => await Task.Run(() => Analyzer.Analyze(Params<AnalyzeParams>(p), ct), ct));
server.On("il/disassemble", async (p, ct) => await Task.Run(() => Il.Disassemble(Params<DisassembleParams>(p), ct), ct));
server.On("xaml/types", async (p, ct) => await Task.Run(() => Xaml.Types(Params<XamlTypesParams>(p), ct), ct));
server.On("heap/retention", async (p, ct) => await Task.Run(() => Heap.Retention(Params<RetentionParams>(p), ct), ct));

await server.RunAsync();

static T Params<T>(JsonElement p)
{
    try
    {
        return p.Deserialize<T>(RpcServer.Json) ?? throw new RpcException(RpcException.InvalidParams, "missing params");
    }
    catch (JsonException e)
    {
        throw new RpcException(RpcException.InvalidParams, e.Message);
    }
}

static class Protocol
{
    /// <summary>Bumped when a method or result changes shape.</summary>
    public const int Version = 4;
}
