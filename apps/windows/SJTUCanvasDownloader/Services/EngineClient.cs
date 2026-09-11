using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.UI.Dispatching;

namespace CanvasDownloader.Services;

/// <summary>An error reported by the engine for one request.</summary>
public sealed class EngineException(string code, string message, int? retryAfterSeconds = null) : Exception(message)
{
    public string Code { get; } = code;

    /// <summary>Seconds after which the school suggests trying again.</summary>
    public int? RetryAfterSeconds { get; } = retryAfterSeconds;

    public bool IsUnauthorized => Code == "unauthorized";
}

/// <summary>
/// Runs sjtu-canvas-engine as a child process and talks newline-delimited
/// JSON-RPC over its stdin/stdout. The engine exits when stdin closes, and a
/// kill-on-close job object ends it if this app crashes.
/// </summary>
public sealed class EngineClient : IAsyncDisposable
{
    public static readonly JsonSerializerOptions Json = new(JsonSerializerDefaults.Web)
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        DictionaryKeyPolicy = null,
        DefaultIgnoreCondition = JsonIgnoreCondition.Never,
    };

    private readonly DispatcherQueue _dispatcher;
    private readonly ConcurrentDictionary<long, TaskCompletionSource<JsonElement>> _pending = new();
    private readonly SemaphoreSlim _writeLock = new(1, 1);
    private readonly StringBuilder _stderrTail = new();
    private Process? _process;
    private JobObject? _job;
    private long _nextId;
    private bool _disposed;

    public EngineClient(DispatcherQueue dispatcher)
    {
        _dispatcher = dispatcher;
    }

    public event Action<LoginStatus>? LoginStatusChanged;
    public event Action<AccountInfo>? AccountChanged;
    public event Action<DownloadInfo>? DownloadChanged;
    public event Action<DownloadProgress>? DownloadProgressed;
    public event Action<string>? DownloadRemoved;
    public event Action<string>? Exited;

    public bool IsRunning => _process is { HasExited: false };

    public void Start(string enginePath, string dataDir)
    {
        var start = new ProcessStartInfo(enginePath)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            StandardInputEncoding = new UTF8Encoding(false),
            StandardOutputEncoding = new UTF8Encoding(false),
            StandardErrorEncoding = new UTF8Encoding(false),
            WorkingDirectory = Path.GetDirectoryName(enginePath)!,
        };
        start.ArgumentList.Add("--data-dir");
        start.ArgumentList.Add(dataDir);

        _process = Process.Start(start) ?? throw new InvalidOperationException("无法启动下载引擎");
        _job = JobObject.TryCreateKillOnClose();
        _job?.Assign(_process);
        _process.StandardInput.AutoFlush = true;
        _process.EnableRaisingEvents = true;
        _process.Exited += (_, _) => OnExited();
        _ = Task.Run(ReadOutputAsync);
        _ = Task.Run(ReadErrorAsync);
    }

    public async Task<T> CallAsync<T>(string method, object? parameters = null, CancellationToken cancellation = default)
    {
        var result = await CallRawAsync(method, parameters, cancellation).ConfigureAwait(false);
        return result.Deserialize<T>(Json) ?? throw new EngineException("failed", $"{method} 返回了空结果");
    }

    public Task CallAsync(string method, object? parameters = null, CancellationToken cancellation = default) =>
        CallRawAsync(method, parameters, cancellation);

    private async Task<JsonElement> CallRawAsync(string method, object? parameters, CancellationToken cancellation)
    {
        var process = _process;
        if (process is null || process.HasExited)
        {
            throw new EngineException("engine_stopped", "下载引擎未运行");
        }

        var id = Interlocked.Increment(ref _nextId);
        var completion = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        _pending[id] = completion;
        var line = JsonSerializer.Serialize(new { id, method, @params = parameters ?? new { } }, Json);
        await _writeLock.WaitAsync(cancellation).ConfigureAwait(false);
        try
        {
            await process.StandardInput.WriteAsync(line + "\n").ConfigureAwait(false);
        }
        catch (Exception error) when (error is IOException or ObjectDisposedException or InvalidOperationException)
        {
            _pending.TryRemove(id, out _);
            throw new EngineException("engine_stopped", "下载引擎连接已断开");
        }
        finally
        {
            _writeLock.Release();
        }

        using var registration = cancellation.Register(() =>
        {
            if (_pending.TryRemove(id, out var pending))
            {
                pending.TrySetCanceled(cancellation);
            }
        });
        return await completion.Task.ConfigureAwait(false);
    }

    private async Task ReadOutputAsync()
    {
        var reader = _process!.StandardOutput;
        while (await reader.ReadLineAsync().ConfigureAwait(false) is { } line)
        {
            if (line.Length == 0)
            {
                continue;
            }

            try
            {
                using var document = JsonDocument.Parse(line);
                var root = document.RootElement;
                if (root.TryGetProperty("id", out var idElement) && idElement.ValueKind == JsonValueKind.Number)
                {
                    if (!_pending.TryRemove(idElement.GetInt64(), out var pending))
                    {
                        continue;
                    }

                    if (root.TryGetProperty("error", out var error))
                    {
                        var code = error.TryGetProperty("code", out var c) ? c.GetString() ?? "failed" : "failed";
                        var message = error.TryGetProperty("message", out var m) ? m.GetString() ?? "未知错误" : "未知错误";
                        int? retry = error.TryGetProperty("retry_after_seconds", out var r) && r.ValueKind == JsonValueKind.Number
                            ? r.GetInt32()
                            : null;
                        pending.TrySetException(new EngineException(code, message, retry));
                    }
                    else
                    {
                        pending.TrySetResult(root.TryGetProperty("result", out var result) ? result.Clone() : default);
                    }
                }
                else if (root.TryGetProperty("method", out var methodElement))
                {
                    Dispatch(methodElement.GetString(), root.TryGetProperty("params", out var p) ? p.Clone() : default);
                }
            }
            catch (JsonException)
            {
                // A malformed line is ignored; the protocol is line-delimited.
            }
        }
    }

    private void Dispatch(string? method, JsonElement parameters)
    {
        switch (method)
        {
            case "login.status":
                if (parameters.Deserialize<LoginStatus>(Json) is { } status)
                {
                    _dispatcher.TryEnqueue(() => LoginStatusChanged?.Invoke(status));
                }
                break;
            case "account.changed":
                if (parameters.Deserialize<AccountInfo>(Json) is { } account)
                {
                    _dispatcher.TryEnqueue(() => AccountChanged?.Invoke(account));
                }
                break;
            case "download.changed":
                if (parameters.Deserialize<DownloadInfo>(Json) is { } download)
                {
                    _dispatcher.TryEnqueue(() => DownloadChanged?.Invoke(download));
                }
                break;
            case "download.progress":
                if (parameters.Deserialize<DownloadProgress>(Json) is { } progress)
                {
                    _dispatcher.TryEnqueue(() => DownloadProgressed?.Invoke(progress));
                }
                break;
            case "download.removed":
                var removed = parameters.GetProperty("id").GetString() ?? "";
                _dispatcher.TryEnqueue(() => DownloadRemoved?.Invoke(removed));
                break;
        }
    }

    private async Task ReadErrorAsync()
    {
        var reader = _process!.StandardError;
        while (await reader.ReadLineAsync().ConfigureAwait(false) is { } line)
        {
            lock (_stderrTail)
            {
                _stderrTail.AppendLine(line);
                if (_stderrTail.Length > 8_000)
                {
                    _stderrTail.Remove(0, _stderrTail.Length - 8_000);
                }
            }
        }
    }

    private void OnExited()
    {
        foreach (var pending in _pending.Values)
        {
            pending.TrySetException(new EngineException("engine_stopped", "下载引擎已退出"));
        }
        _pending.Clear();
        if (_disposed)
        {
            return;
        }

        string detail;
        lock (_stderrTail)
        {
            detail = _stderrTail.ToString().Trim();
        }
        var code = SafeExitCode();
        var message = string.IsNullOrEmpty(detail) ? $"下载引擎意外退出（代码 {code}）" : detail;
        _dispatcher.TryEnqueue(() => Exited?.Invoke(message));
    }

    private int SafeExitCode()
    {
        try
        {
            return _process?.ExitCode ?? -1;
        }
        catch (InvalidOperationException)
        {
            return -1;
        }
    }

    public async ValueTask DisposeAsync()
    {
        if (_disposed)
        {
            return;
        }
        _disposed = true;
        var process = _process;
        if (process is { HasExited: false })
        {
            try
            {
                using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(2));
                await CallRawAsync("engine.shutdown", null, timeout.Token).ConfigureAwait(false);
            }
            catch (Exception)
            {
                // Closing stdin below stops the engine as well.
            }

            try
            {
                process.StandardInput.Close();
                using var exit = new CancellationTokenSource(TimeSpan.FromSeconds(8));
                await process.WaitForExitAsync(exit.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                process.Kill(entireProcessTree: true);
            }
            catch (Exception)
            {
                // The process is already gone.
            }
        }
        _job?.Dispose();
        process?.Dispose();
    }
}
