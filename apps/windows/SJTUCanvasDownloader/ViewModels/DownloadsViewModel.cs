using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using CanvasDownloader.Helpers;
using CanvasDownloader.Services;

namespace CanvasDownloader.ViewModels;

/// <summary>One download task row.</summary>
public sealed partial class DownloadItemViewModel : ObservableObject
{
    public DownloadItemViewModel(DownloadInfo info)
    {
        Info = info;
        Update(info);
    }

    public DownloadInfo Info { get; private set; }

    public string Id => Info.Id;

    [ObservableProperty]
    public partial string Title { get; set; } = "";

    [ObservableProperty]
    public partial string Subtitle { get; set; } = "";

    [ObservableProperty]
    public partial string Glyph { get; set; } = "";

    [ObservableProperty]
    public partial string StatusText { get; set; } = "";

    [ObservableProperty]
    public partial double Progress { get; set; }

    [ObservableProperty]
    public partial bool ShowProgress { get; set; }

    [ObservableProperty]
    public partial bool IsIndeterminate { get; set; }

    [ObservableProperty]
    public partial bool ShowPaused { get; set; }

    [ObservableProperty]
    public partial bool ShowError { get; set; }

    [ObservableProperty]
    public partial bool IsCompleted { get; set; }

    [ObservableProperty]
    public partial bool IsFailed { get; set; }

    [ObservableProperty]
    public partial bool CanPause { get; set; }

    [ObservableProperty]
    public partial bool CanResume { get; set; }

    [ObservableProperty]
    public partial bool CanCancel { get; set; }

    [ObservableProperty]
    public partial bool CanRetry { get; set; }

    [ObservableProperty]
    public partial bool CanRemove { get; set; }

    public void Update(DownloadInfo info)
    {
        Info = info;
        Title = info.DisplayName;
        Subtitle = info.Kind == "video"
            ? $"{info.CourseName} · 课堂录像"
            : $"{info.CourseName} · 课程文件";
        Glyph = info.Kind == "video" ? "\uE714" : Format.FileGlyph(null, info.Title);
        IsCompleted = info.Status == "completed";
        IsFailed = info.Status == "failed";
        CanPause = info.Status is "queued" or "downloading";
        CanResume = info.Status == "paused";
        CanCancel = info.IsUnfinished || info.Status == "failed";
        CanRetry = info.IsStopped;
        CanRemove = !info.IsUnfinished;
        ShowProgress = info.Status is "downloading" or "paused" or "queued" or "failed" && (info.Received > 0 || info.Status == "downloading");
        ShowPaused = info.Status is "paused" or "queued";
        ShowError = info.Status == "failed";
        Refresh();
    }

    /// <summary>The name screen readers and UI Automation read for the row.</summary>
    public override string ToString() => Title;

    public void Apply(DownloadProgress progress)
    {
        if (Info.Status != "downloading")
        {
            return;
        }
        Info.Received = progress.Received;
        Info.Total = progress.Total ?? Info.Total;
        Info.Speed = progress.Speed;
        Refresh();
    }

    private void Refresh()
    {
        var info = Info;
        IsIndeterminate = info.Status == "downloading" && info.Total is null;
        Progress = info.Total is > 0 ? Math.Min(100, info.Received * 100.0 / info.Total.Value) : 0;
        var amount = info.Total is { } total
            ? $"{Format.Size(info.Received)} / {Format.Size(total)}"
            : Format.Size(info.Received);
        StatusText = info.Status switch
        {
            "downloading" => string.Join(" · ", new[] { "下载中", amount, Format.Speed(info.Speed), Format.Remaining(info.Received, info.Total, info.Speed) }.Where(part => part.Length > 0)),
            "queued" => info.Error is { Length: > 0 } note ? $"排队中 · {note}" : "排队中",
            "paused" => info.Received > 0 ? $"已暂停 · {amount}" : "已暂停",
            "completed" => $"已完成 · {Format.Size(info.Total ?? info.Received)}" + (info.CompletedAt is { } done ? $" · {Format.RelativeDate(done)}" : ""),
            "failed" => $"失败：{info.Error}",
            "cancelled" => "已取消",
            _ => info.Status,
        };
    }
}

/// <summary>The download list, kept in step with engine notifications.</summary>
public sealed partial class DownloadsViewModel : ObservableObject
{
    private EngineClient? _engine;
    private int _generation;
    private CancellationTokenSource? _countsDelay;

    public ObservableCollection<DownloadItemViewModel> Items { get; } = [];

    /// <summary>all | active | completed | failed</summary>
    [ObservableProperty]
    public partial string Filter { get; set; } = "all";

    [ObservableProperty]
    public partial DownloadCounts Counts { get; set; } = new();

    [ObservableProperty]
    public partial bool IsEmpty { get; set; }

    [ObservableProperty]
    public partial string? Error { get; set; }

    public void Attach()
    {
        Detach();
        _engine = AppHost.Engine;
        if (_engine is null)
        {
            return;
        }
        _engine.DownloadChanged += OnChanged;
        _engine.DownloadProgressed += OnProgress;
        _engine.DownloadRemoved += OnRemoved;
    }

    public void Detach()
    {
        if (_engine is null)
        {
            return;
        }
        _engine.DownloadChanged -= OnChanged;
        _engine.DownloadProgressed -= OnProgress;
        _engine.DownloadRemoved -= OnRemoved;
        _engine = null;
    }

    public async Task LoadAsync()
    {
        if (!AppHost.IsReady)
        {
            return;
        }
        var generation = ++_generation;
        Error = null;
        try
        {
            var list = await AppHost.RequireEngine().CallAsync<DownloadList>("downloads.list", new { filter = Filter });
            if (generation != _generation)
            {
                return;
            }
            Counts = list.Counts;
            Synchronize(list.Items);
        }
        catch (EngineException error)
        {
            Error = error.Message;
        }
        IsEmpty = Items.Count == 0;
    }

    /// <summary>Updates rows in place so selection and scroll position survive.</summary>
    private void Synchronize(List<DownloadInfo> downloads)
    {
        var existing = Items.ToDictionary(item => item.Id);
        for (var index = 0; index < downloads.Count; index++)
        {
            var download = downloads[index];
            if (existing.Remove(download.Id, out var item))
            {
                item.Update(download);
                var current = Items.IndexOf(item);
                if (current != index)
                {
                    Items.Move(current, index);
                }
            }
            else
            {
                Items.Insert(index, new DownloadItemViewModel(download));
            }
        }
        foreach (var stale in existing.Values)
        {
            Items.Remove(stale);
        }
    }

    private bool Matches(DownloadInfo info) => Filter switch
    {
        "active" => info.IsUnfinished,
        "completed" => info.Status == "completed",
        "failed" => info.IsStopped,
        _ => true,
    };

    private void OnChanged(DownloadInfo info)
    {
        var item = Items.FirstOrDefault(candidate => candidate.Id == info.Id);
        if (Matches(info))
        {
            if (item is null)
            {
                // New tasks belong to the newest batch, at the top in queue order.
                var position = 0;
                while (position < Items.Count && Items[position].Info.CreatedAt > info.CreatedAt)
                {
                    position++;
                }
                while (position < Items.Count && Items[position].Info.CreatedAt == info.CreatedAt)
                {
                    position++;
                }
                Items.Insert(position, new DownloadItemViewModel(info));
            }
            else
            {
                item.Update(info);
            }
        }
        else if (item is not null)
        {
            Items.Remove(item);
        }
        IsEmpty = Items.Count == 0;
        RefreshCountsSoon();
    }

    private void OnProgress(DownloadProgress progress)
    {
        Items.FirstOrDefault(item => item.Id == progress.Id)?.Apply(progress);
    }

    private void OnRemoved(string id)
    {
        if (Items.FirstOrDefault(candidate => candidate.Id == id) is { } item)
        {
            Items.Remove(item);
        }
        IsEmpty = Items.Count == 0;
        RefreshCountsSoon();
    }

    private async void RefreshCountsSoon()
    {
        _countsDelay?.Cancel();
        var delay = new CancellationTokenSource();
        _countsDelay = delay;
        try
        {
            await Task.Delay(300, delay.Token);
            Counts = (await AppHost.RequireEngine().CallAsync<DownloadList>("downloads.list", new { limit = 1 })).Counts;
        }
        catch (Exception error) when (error is TaskCanceledException or EngineException)
        {
        }
    }
}
