using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Bayonet.Diagnostics;

/// <summary>A failure reported to the caller as a JSON-RPC error.</summary>
sealed class RpcException(int code, string message) : Exception(message)
{
    public const int InvalidParams = -32602;
    public const int MethodNotFound = -32601;
    public const int RequestCancelled = -32800;
    public const int Failed = -32000;

    public int Code { get; } = code;
}

/// <summary>
/// JSON-RPC 2.0 over <c>Content-Length</c> framing (as LSP), on stdin and stdout.
/// Each request runs on its own task, so <c>$/cancelRequest</c> can reach it.
/// </summary>
sealed class RpcServer(Stream input, Stream output)
{
    public static readonly JsonSerializerOptions Json = new(JsonSerializerDefaults.Web);

    readonly SemaphoreSlim writeLock = new(1, 1);
    readonly Dictionary<string, CancellationTokenSource> running = [];
    readonly Dictionary<string, Func<JsonElement, CancellationToken, Task<object?>>> methods = [];
    readonly BufferedStream reader = new(input, 64 * 1024);

    public void On(string method, Func<JsonElement, CancellationToken, Task<object?>> handler) => methods[method] = handler;

    public async Task RunAsync()
    {
        var tasks = new List<Task>();
        while (await ReadAsync() is { } body)
        {
            JsonElement message;
            try
            {
                message = JsonDocument.Parse(body).RootElement;
            }
            catch (JsonException e)
            {
                await Log($"ignoring a message that is not JSON: {e.Message}");
                continue;
            }
            var method = message.TryGetProperty("method", out var m) ? m.GetString() : null;
            var hasId = message.TryGetProperty("id", out var id);
            var parameters = message.TryGetProperty("params", out var p) ? p : default;
            if (method == "$/cancelRequest")
            {
                if (parameters.ValueKind == JsonValueKind.Object && parameters.TryGetProperty("id", out var cancelled))
                {
                    lock (running)
                    {
                        if (running.TryGetValue(cancelled.GetRawText(), out var cts)) cts.Cancel();
                    }
                }
                continue;
            }
            if (method == "exit") break;
            if (method is null || !hasId) continue;
            tasks.RemoveAll(t => t.IsCompleted);
            tasks.Add(Task.Run(() => Dispatch(id, method, parameters)));
        }
        lock (running)
        {
            foreach (var cts in running.Values) cts.Cancel();
        }
        await Task.WhenAll(tasks);
    }

    async Task Dispatch(JsonElement id, string method, JsonElement parameters)
    {
        var key = id.GetRawText();
        using var cts = new CancellationTokenSource();
        lock (running) running[key] = cts;
        JsonObject response = new() { ["jsonrpc"] = "2.0", ["id"] = JsonNode.Parse(key) };
        try
        {
            if (!methods.TryGetValue(method, out var handler))
                throw new RpcException(RpcException.MethodNotFound, $"unknown method {method}");
            var result = await handler(parameters, cts.Token);
            response["result"] = JsonSerializer.SerializeToNode(result, Json);
        }
        catch (Exception e)
        {
            var (code, text) = e switch
            {
                RpcException r => (r.Code, r.Message),
                OperationCanceledException when cts.IsCancellationRequested => (RpcException.RequestCancelled, "cancelled"),
                _ => (RpcException.Failed, e.Message),
            };
            if (code == RpcException.Failed) await Log($"{method} failed: {e}");
            response["error"] = new JsonObject { ["code"] = code, ["message"] = text };
        }
        finally
        {
            lock (running) running.Remove(key);
        }
        await WriteAsync(Encoding.UTF8.GetBytes(response.ToJsonString(Json)));
    }

    async Task<byte[]?> ReadAsync()
    {
        int? length = null;
        while (true)
        {
            var line = await ReadLineAsync();
            if (line is null) return null;
            if (line.Length == 0)
            {
                if (length is null) continue;
                break;
            }
            var colon = line.IndexOf(':');
            if (colon > 0 && line[..colon].Trim().Equals("Content-Length", StringComparison.OrdinalIgnoreCase))
                length = int.Parse(line[(colon + 1)..].Trim());
        }
        var body = new byte[length.Value];
        await reader.ReadExactlyAsync(body);
        return body;
    }

    async Task<string?> ReadLineAsync()
    {
        var bytes = new List<byte>(64);
        var one = new byte[1];
        while (true)
        {
            if (await reader.ReadAsync(one) == 0) return bytes.Count == 0 ? null : Encoding.ASCII.GetString([.. bytes]);
            if (one[0] == (byte)'\n') break;
            bytes.Add(one[0]);
        }
        if (bytes.Count > 0 && bytes[^1] == (byte)'\r') bytes.RemoveAt(bytes.Count - 1);
        return Encoding.ASCII.GetString([.. bytes]);
    }

    async Task WriteAsync(byte[] body)
    {
        var header = Encoding.ASCII.GetBytes($"Content-Length: {body.Length}\r\n\r\n");
        await writeLock.WaitAsync();
        try
        {
            await output.WriteAsync(header);
            await output.WriteAsync(body);
            await output.FlushAsync();
        }
        finally
        {
            writeLock.Release();
        }
    }

    public static Task Log(string text) => Console.Error.WriteLineAsync(text);
}
