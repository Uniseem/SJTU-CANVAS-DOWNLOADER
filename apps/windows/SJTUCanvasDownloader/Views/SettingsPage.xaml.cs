using CanvasDownloader.Helpers;
using CanvasDownloader.Services;
using CanvasDownloader.ViewModels;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;

namespace CanvasDownloader.Views;

public sealed partial class SettingsPage : Page
{
    public SettingsPage()
    {
        InitializeComponent();
    }

    public SettingsViewModel ViewModel { get; } = new();

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        AppHost.SettingsChanged += OnChanged;
        AppHost.AccountChanged += OnChanged;
        ViewModel.Load();
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        base.OnNavigatedFrom(e);
        AppHost.SettingsChanged -= OnChanged;
        AppHost.AccountChanged -= OnChanged;
    }

    private void OnChanged() => ViewModel.Load();

    private async void OnChangeFolder(object sender, RoutedEventArgs e)
    {
        if (await ShellService.PickFolderAsync() is { } folder)
        {
            await ViewModel.ChangeDownloadDirAsync(folder);
        }
    }

    private async void OnResetFolder(object sender, RoutedEventArgs e) => await ViewModel.ResetDownloadDirAsync();

    private void OnOpenFolder(object sender, RoutedEventArgs e)
    {
        if (ViewModel.DownloadDir is { Length: > 0 } folder)
        {
            Directory.CreateDirectory(folder);
            ShellService.Reveal(folder);
        }
    }

    private void OnOpenData(object sender, RoutedEventArgs e) => ShellService.Reveal(AppHost.DataDir);

    private async void OnSaveProxy(object sender, RoutedEventArgs e) => await ViewModel.SaveProxyAsync();

    private async void OnSignOut(object sender, RoutedEventArgs e)
    {
        var running = AppHost.Engine is null ? 0 : await RunningCountAsync();
        var message = running > 0
            ? $"还有 {running} 个下载任务未完成，它们会在下次登录后继续。保存在本机的登录状态会被删除。"
            : "保存在本机的登录状态会被删除，下次使用需要重新扫码。";
        if (!await Dialogs.ConfirmAsync(XamlRoot, "退出 Canvas 登录？", message, "退出登录"))
        {
            return;
        }
        try
        {
            await AppHost.SignOutAsync();
        }
        catch (EngineException error)
        {
            await Dialogs.ShowErrorAsync(XamlRoot, "无法退出登录", error.Message);
        }
    }

    private static async Task<long> RunningCountAsync()
    {
        try
        {
            return (await AppHost.RequireEngine().CallAsync<DownloadList>("downloads.list", new { limit = 1 })).Counts.Running;
        }
        catch (EngineException)
        {
            return 0;
        }
    }
}
