using System.Diagnostics;
using System.Diagnostics.Tracing;
using Microsoft.Diagnostics.NETCore.Client;

namespace Bayonet.Diagnostics;

record AttachParams(int Pid, int Seconds, string Output);

record LaunchParams(
    string Program,
    string[]? Args,
    string? Cwd,
    Dictionary<string, string>? Env,
    int Seconds,
    string Output,
    bool StopWhenDone = true);

record Recording(string Path, int Pid, double Seconds, int? ExitCode);

/// <summary>Records EventPipe traces: CPU samples, GC, JIT and exception events, with rundown.</summary>
static class Recorder
{
    /// <summary>
    /// dotnet-trace's cpu-sampling runtime keywords, plus the IL-to-native maps that
    /// let samples be mapped to source lines.
    /// </summary>
    const long RuntimeKeywords = 0x14C14FCCBD | 0x20000;

    static readonly EventPipeProvider[] Providers =
    [
        new("Microsoft-DotNETCore-SampleProfiler", EventLevel.Informational),
        new("Microsoft-Windows-DotNETRuntime", EventLevel.Informational, RuntimeKeywords),
    ];

    public static async Task<Recording> Attach(AttachParams p, CancellationToken ct)
    {
        var client = new DiagnosticsClient(p.Pid);
        var started = Stopwatch.StartNew();
        await Record(client, p.Output, Delay(p.Seconds, ct), ct);
        return new Recording(p.Output, p.Pid, started.Elapsed.TotalSeconds, null);
    }

    /// <summary>
    /// Starts <c>Program</c> suspended on a diagnostic port, so the trace covers its startup,
    /// and records until it exits or <c>Seconds</c> pass.
    /// </summary>
    public static async Task<Recording> Launch(LaunchParams p, CancellationToken ct)
    {
        var port = OperatingSystem.IsWindows()
            ? $"bayonet-diag-{Environment.ProcessId}-{Guid.NewGuid():N}"
            // Socket paths are limited to about 100 bytes; macOS's per-user temp folder alone takes half.
            : $"/tmp/bayonet-diag-{Environment.ProcessId}-{Guid.NewGuid().ToString("N")[..12]}.sock";
        var start = new ProcessStartInfo(p.Program)
        {
            WorkingDirectory = p.Cwd ?? Environment.CurrentDirectory,
            UseShellExecute = false,
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };
        foreach (var a in p.Args ?? []) start.ArgumentList.Add(a);
        foreach (var (k, v) in p.Env ?? []) start.Environment[k] = v;
        start.Environment["DOTNET_DiagnosticPorts"] = port;

        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(ct);
        timeout.CancelAfter(TimeSpan.FromSeconds(30));
        var connecting = DiagnosticsClientConnector.FromDiagnosticPort(port, timeout.Token);
        using var process = Process.Start(start) ?? throw new RpcException(RpcException.Failed, $"cannot start {p.Program}");
        process.OutputDataReceived += (_, e) => { if (e.Data is not null) Console.Error.WriteLine($"[app] {e.Data}"); };
        process.ErrorDataReceived += (_, e) => { if (e.Data is not null) Console.Error.WriteLine($"[app] {e.Data}"); };
        process.BeginOutputReadLine();
        process.BeginErrorReadLine();
        var started = Stopwatch.StartNew();
        try
        {
            await using var connector = await connecting
                ?? throw new RpcException(RpcException.Failed, $"{p.Program} did not connect to the diagnostic port");
            var exited = process.WaitForExitAsync(ct);
            var until = Task.WhenAny(exited, Delay(p.Seconds, ct));
            await Record(connector.Instance, p.Output, until, ct, resume: true);
        }
        finally
        {
            if (p.StopWhenDone && !process.HasExited) process.Kill(entireProcessTree: true);
        }
        return new Recording(p.Output, process.Id, started.Elapsed.TotalSeconds, process.HasExited ? process.ExitCode : null);
    }

    static Task Delay(int seconds, CancellationToken ct) =>
        Task.Delay(TimeSpan.FromSeconds(Math.Clamp(seconds, 1, 3600)), ct);

    static async Task Record(DiagnosticsClient client, string output, Task until, CancellationToken ct, bool resume = false)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(output))!);
        using var session = await client.StartEventPipeSessionAsync(Providers, requestRundown: true, circularBufferMB: 256, ct);
        if (resume) client.ResumeRuntime();
        var completed = false;
        try
        {
            await using (var file = File.Create(output))
            {
                var copy = session.EventStream.CopyToAsync(file, CancellationToken.None);
                try
                {
                    await Task.WhenAny(until, copy);
                    ct.ThrowIfCancellationRequested();
                }
                finally
                {
                    // Stopping flushes the rundown (method names, IL maps) and ends the stream.
                    // A process that already exited has ended it already.
                    try
                    {
                        await session.StopAsync(CancellationToken.None);
                    }
                    catch (Exception e) when (e is ServerNotAvailableException or EndOfStreamException or IOException or TimeoutException)
                    {
                    }
                    await copy;
                }
            }
            completed = true;
        }
        finally
        {
            if (!completed) File.Delete(output);
        }
    }
}
