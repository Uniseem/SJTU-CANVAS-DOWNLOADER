using CanvasDownloader.Helpers;
using CanvasDownloader.Services;
using CanvasDownloader.ViewModels;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;

namespace CanvasDownloader.Views;

public sealed partial class DownloadsPage : Page
{
    public DownloadsPage()
    {
        InitializeComponent();
        ViewModel.PropertyChanged += (_, args) =>
        {
            if (args.PropertyName == nameof(DownloadsViewModel.Counts))
            {
                UpdateFilterLabels();
            }
        };
    }

    public DownloadsViewModel ViewModel { get; } = new();

    protected override async void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        AppHost.EngineStateChanged += OnEngineStateChanged;
        ViewModel.Attach();
        await ViewModel.LoadAsync();
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        base.OnNavigatedFrom(e);
        AppHost.EngineStateChanged -= OnEngineStateChanged;
        ViewModel.Detach();
    }

    private async void OnEngineStateChanged()
    {
        ViewModel.Attach();
        await ViewModel.LoadAsync();
    }

    private async void OnFilterChanged(SelectorBar sender, SelectorBarSelectionChangedEventArgs args)
    {
        ViewModel.Filter = sender.SelectedItem?.Tag as string ?? "all";
        await ViewModel.LoadAsync();
    }

    private void UpdateFilterLabels()
    {
        var counts = ViewModel.Counts;
        AllFilter.Text = counts.All > 0 ? $"全部 {counts.All}" : "全部";
        ActiveFilter.Text = counts.Active > 0 ? $"进行中 {counts.Active}" : "进行中";
        CompletedFilter.Text = counts.Completed > 0 ? $"已完成 {counts.Completed}" : "已完成";
        FailedFilter.Text = counts.Failed > 0 ? $"失败与取消 {counts.Failed}" : "失败与取消";
    }

    public bool CanPauseAll(DownloadCounts counts) => counts.Running > 0;

    public bool CanResumeAll(DownloadCounts counts) => counts.Active > counts.Running;

    public bool CanClear(DownloadCounts counts) => counts.Completed > 0;

    public string EmptyTitle(string filter) => filter switch
    {
        "active" => "没有进行中的下载",
        "completed" => "还没有完成的下载",
        "failed" => "没有失败或取消的下载",
        _ => "还没有下载任务",
    };

    private static DownloadItemViewModel? Item(object sender) =>
        (sender as FrameworkElement)?.Tag as DownloadItemViewModel;

    private async Task RunAsync(object sender, string method)
    {
        if (Item(sender) is not { } item)
        {
            return;
        }
        try
        {
            await AppHost.RequireEngine().CallAsync(method, new { id = item.Id });
        }
        catch (EngineException error)
        {
            await Dialogs.ShowErrorAsync(XamlRoot, "操作失败", error.Message);
        }
    }

    private async void OnPause(object sender, RoutedEventArgs e) => await RunAsync(sender, "downloads.pause");

    private async void OnResume(object sender, RoutedEventArgs e) => await RunAsync(sender, "downloads.resume");

    private async void OnRetry(object sender, RoutedEventArgs e) => await RunAsync(sender, "downloads.retry");

    private async void OnRemove(object sender, RoutedEventArgs e) => await RunAsync(sender, "downloads.remove");

    private async void OnCancel(object sender, RoutedEventArgs e)
    {
        if (Item(sender) is not { } item)
        {
            return;
        }
        var partial = item.Info.Received > 0 ? "已下载的部分会被删除，" : "";
        if (await Dialogs.ConfirmAsync(XamlRoot, "取消这个下载？", $"“{item.Title}”{partial}之后可以重新下载。", "取消下载", destructive: true))
        {
            await RunAsync(sender, "downloads.cancel");
        }
    }

    private void OnOpen(object sender, RoutedEventArgs e)
    {
        if (Item(sender)?.Info.FilePath is { } path && File.Exists(path))
        {
            ShellService.Open(path);
        }
    }

    private void OnReveal(object sender, RoutedEventArgs e)
    {
        if (Item(sender)?.Info is { } info)
        {
            ShellService.Reveal(info.FilePath is { } path && File.Exists(path) ? path : info.FilePath ?? info.Destination);
        }
    }

    private async void OnPauseAll(object sender, RoutedEventArgs e) => await CallAsync("downloads.pauseAll");

    private async void OnResumeAll(object sender, RoutedEventArgs e) => await CallAsync("downloads.resumeAll");

    private async void OnClearCompleted(object sender, RoutedEventArgs e) => await CallAsync("downloads.clearCompleted");

    private async Task CallAsync(string method)
    {
        try
        {
            await AppHost.RequireEngine().CallAsync(method);
        }
        catch (EngineException error)
        {
            await Dialogs.ShowErrorAsync(XamlRoot, "操作失败", error.Message);
        }
    }

    private void OnOpenFolder(object sender, RoutedEventArgs e)
    {
        if (AppHost.Settings?.Preferences.DownloadDir is { Length: > 0 } folder)
        {
            Directory.CreateDirectory(folder);
            ShellService.Reveal(folder);
        }
    }

    private void OnBrowseCourses(object sender, RoutedEventArgs e) => App.Window.Navigate(typeof(CoursesPage));
}
