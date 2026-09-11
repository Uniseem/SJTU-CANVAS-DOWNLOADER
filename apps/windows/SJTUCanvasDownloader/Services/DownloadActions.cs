namespace CanvasDownloader.Services;

/// <summary>Queues downloads, asking for a folder first when the user wants that.</summary>
public static class DownloadActions
{
    /// <summary>The result, or null when the user cancelled the folder picker.</summary>
    public static async Task<CreateResult?> CreateAsync(IReadOnlyList<NewDownload> items)
    {
        string? destination = null;
        if (AppHost.Settings?.Preferences.AskDestination == true)
        {
            destination = await ShellService.PickFolderAsync();
            if (destination is null)
            {
                return null;
            }
        }
        var result = new CreateResult();
        // The engine accepts at most 500 items per request.
        foreach (var batch in items.Chunk(500))
        {
            var part = await AppHost.RequireEngine().CallAsync<CreateResult>("downloads.create", new
            {
                items = batch,
                destination,
            });
            result.Created.AddRange(part.Created);
            result.Skipped.AddRange(part.Skipped);
        }
        return result;
    }

    /// <summary>"已添加 4 个下载任务，2 个已经下载过" plus what else was skipped and why.</summary>
    public static string Summary(CreateResult result)
    {
        var parts = new List<string>
        {
            result.Created.Count > 0 ? $"已添加 {result.Created.Count} 个下载任务" : "没有添加新的下载任务",
        };
        var downloaded = result.Skipped.Count(item => item.Reason == "downloaded");
        var queued = result.Skipped.Count(item => item.Reason == "queued");
        if (downloaded > 0)
        {
            parts.Add($"{downloaded} 个已经下载过");
        }
        if (queued > 0)
        {
            parts.Add($"{queued} 个已在下载队列中");
        }
        var text = string.Join("，", parts);
        if (result.Skipped.FirstOrDefault(item => item.Reason is not ("downloaded" or "queued")) is { } other)
        {
            text += $"。{other.Message}";
        }
        return text;
    }
}
