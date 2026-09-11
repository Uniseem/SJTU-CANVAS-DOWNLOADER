using CommunityToolkit.Mvvm.ComponentModel;
using CanvasDownloader.Services;

namespace CanvasDownloader.ViewModels;

/// <summary>Preferences shared with the engine (and therefore with the macOS app).</summary>
public sealed partial class SettingsViewModel : ObservableObject
{
    private bool _loading;

    [ObservableProperty]
    public partial string DownloadDir { get; set; } = "";

    [ObservableProperty]
    public partial bool IsDefaultDownloadDir { get; set; }

    [ObservableProperty]
    public partial bool AskDestination { get; set; }

    [ObservableProperty]
    public partial double Concurrency { get; set; } = 3;

    [ObservableProperty]
    public partial int ConcurrencyMax { get; set; } = 8;

    [ObservableProperty]
    public partial bool Slides { get; set; }

    [ObservableProperty]
    public partial bool Teacher { get; set; }

    [ObservableProperty]
    public partial bool Composite { get; set; }

    /// <summary>0 跟随系统, 1 不使用代理, 2 自定义.</summary>
    [ObservableProperty]
    public partial int ProxyModeIndex { get; set; }

    [ObservableProperty]
    public partial string ProxyUrl { get; set; } = "";

    [ObservableProperty]
    public partial bool IsCustomProxy { get; set; }

    [ObservableProperty]
    public partial string? Error { get; set; }

    [ObservableProperty]
    public partial string AccountName { get; set; } = "";

    [ObservableProperty]
    public partial string AccountDetail { get; set; } = "";

    [ObservableProperty]
    public partial bool IsSignedIn { get; set; }

    [ObservableProperty]
    public partial string VersionText { get; set; } = "";

    [ObservableProperty]
    public partial string DataDir { get; set; } = "";

    [ObservableProperty]
    public partial bool IsDemo { get; set; }

    public void Load()
    {
        _loading = true;
        try
        {
            var settings = AppHost.Settings;
            if (settings is not null)
            {
                var preferences = settings.Preferences;
                DownloadDir = preferences.DownloadDir;
                IsDefaultDownloadDir = string.Equals(preferences.DownloadDir, settings.DefaultDownloadDir, StringComparison.OrdinalIgnoreCase);
                AskDestination = preferences.AskDestination;
                ConcurrencyMax = settings.ConcurrencyMax;
                Concurrency = preferences.Concurrency;
                Slides = preferences.DefaultTracks.Contains("slides");
                Teacher = preferences.DefaultTracks.Contains("teacher");
                Composite = preferences.DefaultTracks.Contains("composite");
                ProxyModeIndex = preferences.Proxy.Mode switch { "direct" => 1, "custom" => 2, _ => 0 };
                ProxyUrl = preferences.Proxy.Url ?? "";
                IsCustomProxy = ProxyModeIndex == 2;
                DataDir = settings.DataDir;
                IsDemo = settings.FakeSchool;
            }
            var account = AppHost.Account;
            IsSignedIn = account.Authenticated;
            AccountName = account.Profile?.Name ?? "未登录";
            AccountDetail = account.Authenticated
                ? (account.Persisted ? "登录状态已加密保存在本机，密钥在 Windows 凭据管理器中" : "登录状态仅在本次运行期间有效（无法使用 Windows 凭据管理器）")
                : "扫码登录后即可浏览课程和下载";
            var version = typeof(SettingsViewModel).Assembly.GetName().Version?.ToString(3) ?? "";
            VersionText = AppHost.EngineVersion is { Length: > 0 } engine ? $"应用 {version} · 引擎 {engine}" : $"应用 {version}";
        }
        finally
        {
            _loading = false;
        }
    }

    partial void OnAskDestinationChanged(bool value) => _ = SaveAsync();

    partial void OnConcurrencyChanged(double value)
    {
        if (!double.IsNaN(value))
        {
            _ = SaveAsync();
        }
    }

    partial void OnSlidesChanged(bool value) => SaveTracks();

    partial void OnTeacherChanged(bool value) => SaveTracks();

    partial void OnCompositeChanged(bool value) => SaveTracks();

    partial void OnProxyModeIndexChanged(int value)
    {
        IsCustomProxy = value == 2;
        // A custom proxy is saved with its button once the address is typed.
        if (value != 2)
        {
            _ = SaveAsync();
        }
    }

    private void SaveTracks()
    {
        if (_loading)
        {
            return;
        }
        if (!Slides && !Teacher && !Composite)
        {
            Error = "至少选择一个默认下载的画面";
            Load();
            return;
        }
        _ = SaveAsync();
    }

    public Task ChangeDownloadDirAsync(string folder)
    {
        DownloadDir = folder;
        return SaveAsync();
    }

    public Task ResetDownloadDirAsync()
    {
        DownloadDir = AppHost.Settings?.DefaultDownloadDir ?? DownloadDir;
        return SaveAsync();
    }

    public Task SaveProxyAsync() => SaveAsync();

    private async Task SaveAsync()
    {
        if (_loading || AppHost.Settings is null)
        {
            return;
        }
        var preferences = AppHost.Settings.Preferences.Clone();
        preferences.DownloadDir = DownloadDir;
        preferences.AskDestination = AskDestination;
        preferences.Concurrency = (int)Math.Clamp(Math.Round(Concurrency), 1, ConcurrencyMax);
        preferences.DefaultTracks = new[] { ("slides", Slides), ("teacher", Teacher), ("composite", Composite) }
            .Where(track => track.Item2)
            .Select(track => track.Item1)
            .ToList();
        preferences.Proxy = ProxyModeIndex switch
        {
            1 => new ProxySettings { Mode = "direct" },
            2 => new ProxySettings { Mode = "custom", Url = ProxyUrl.Trim() },
            _ => new ProxySettings { Mode = "system" },
        };
        try
        {
            var updated = await AppHost.RequireEngine().CallAsync<SettingsInfo>("settings.update", new { preferences });
            Error = null;
            AppHost.UpdateSettings(updated);
        }
        catch (EngineException error)
        {
            Error = error.Message;
        }
        Load();
    }
}
