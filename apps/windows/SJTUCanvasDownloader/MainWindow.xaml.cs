using System.Runtime.InteropServices;
using CanvasDownloader.Helpers;
using CanvasDownloader.Services;
using CanvasDownloader.Views;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media.Animation;
using Microsoft.UI.Xaml.Navigation;
using Microsoft.Windows.AppNotifications;
using Microsoft.Windows.AppNotifications.Builder;
using Windows.Graphics;

namespace CanvasDownloader;

public sealed partial class MainWindow : Window
{
    private readonly DispatcherQueueTimer _countsTimer;
    private EngineClient? _engine;
    private long _running;
    private bool _wasRunning;
    private int _completedInBatch;
    private int _failedInBatch;
    private bool _notificationsRegistered;
    private bool _closing;

    public MainWindow()
    {
        App.Attach(this);
        InitializeComponent();
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);
        AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall;
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "AppIcon.ico"));
        RestorePlacement();

        _countsTimer = DispatcherQueue.CreateTimer();
        _countsTimer.Interval = TimeSpan.FromMilliseconds(500);
        _countsTimer.IsRepeating = false;
        _countsTimer.Tick += async (_, _) => await RefreshCountsAsync();

        AppHost.EngineStateChanged += OnEngineStateChanged;
        AppHost.AccountChanged += OnAccountChanged;
        RegisterNotifications();
        AppWindow.Closing += OnClosing;
        Closed += OnClosed;
        NavView.Loaded += (_, _) =>
        {
            if (NavView.SettingsItem is NavigationViewItem settings)
            {
                settings.Content = "设置";
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(settings, "设置");
            }
        };
        ContentFrame.NavigationFailed += (_, args) =>
        {
            args.Handled = true;
            AppLog.Error("navigation failed", args.Exception);
            ShowError("无法打开页面", args.Exception.Message);
        };
        ShowStarting();
    }

    public void Navigate(Type page, object? parameter = null, bool clearHistory = false)
    {
        if (ContentFrame.CurrentSourcePageType == page && parameter is null && !clearHistory)
        {
            return;
        }
        NavigationTransitionInfo transition = page == typeof(CoursePage)
            ? new DrillInNavigationTransitionInfo()
            : new EntranceNavigationTransitionInfo();
        try
        {
            ContentFrame.Navigate(page, parameter, transition);
            if (clearHistory)
            {
                ContentFrame.BackStack.Clear();
                AppTitleBar.IsBackButtonEnabled = false;
            }
        }
        catch (Exception error)
        {
            AppLog.Error($"navigate {page.Name}", error);
            ShowError("无法打开页面", error.Message);
        }
    }

    public void ShowError(string title, string message)
    {
        EngineInfoBar.Title = title;
        EngineInfoBar.Message = message;
        EngineInfoBar.Severity = InfoBarSeverity.Error;
        EngineRetryButton.Visibility = Visibility.Collapsed;
        EngineInfoBar.IsClosable = true;
        EngineInfoBar.IsOpen = true;
    }

    public void BringToFront()
    {
        if (AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Minimized } presenter)
        {
            presenter.Restore();
        }
        Activate();
        SetForegroundWindow(App.WindowHandle);
    }

    private void ShowStarting()
    {
        EngineInfoBar.Title = "正在启动下载引擎";
        EngineInfoBar.Message = "";
        EngineInfoBar.Severity = InfoBarSeverity.Informational;
        EngineRetryButton.Visibility = Visibility.Collapsed;
        EngineInfoBar.IsClosable = false;
        EngineInfoBar.IsOpen = true;
    }

    private void OnEngineStateChanged()
    {
        if (AppHost.IsReady)
        {
            EngineInfoBar.IsOpen = false;
            AttachEngine(AppHost.Engine!);
            _countsTimer.Start();
            return;
        }

        EngineInfoBar.Title = "下载引擎未运行";
        EngineInfoBar.Message = AppHost.StartupError ?? "";
        EngineInfoBar.Severity = InfoBarSeverity.Error;
        EngineRetryButton.Visibility = Visibility.Visible;
        EngineInfoBar.IsClosable = false;
        EngineInfoBar.IsOpen = true;
        RunningBadge.Visibility = Visibility.Collapsed;
    }

    private void AttachEngine(EngineClient engine)
    {
        if (_engine is not null)
        {
            _engine.DownloadChanged -= OnDownloadChanged;
            _engine.DownloadRemoved -= OnDownloadRemoved;
        }
        _engine = engine;
        engine.DownloadChanged += OnDownloadChanged;
        engine.DownloadRemoved += OnDownloadRemoved;
    }

    /// <summary>Signed in: the navigation pane and the course list; signed out: the login page.</summary>
    private void OnAccountChanged()
    {
        var account = AppHost.Account;
        var signedIn = AppHost.IsSignedIn;
        var name = account.Profile?.Name ?? "";
        AccountItem.Content = name.Length > 0 ? name : "账户";
        AccountPicture.DisplayName = name;
        AccountPicture.Initials = name.Length > 0 ? name[..1] : "";
        AccountNameText.Text = name;
        AccountIdText.Text = account.Profile is { } profile ? $"Canvas 用户 ID {profile.Id}" : "";
        if (!AppHost.IsReady)
        {
            return;
        }
        NavView.IsPaneVisible = signedIn;
        AppTitleBar.IsPaneToggleButtonVisible = signedIn;
        var page = ContentFrame.CurrentSourcePageType;
        if (signedIn && (page is null || page == typeof(LoginPage)))
        {
            Navigate(typeof(CoursesPage), clearHistory: true);
        }
        else if (signedIn)
        {
            // Signed in from another page (settings opened from the login
            // page): going back must not return to the finished login.
            RemoveFromHistory(typeof(LoginPage));
        }
        else if (page != typeof(LoginPage))
        {
            Navigate(typeof(LoginPage), clearHistory: true);
        }
    }

    private void RemoveFromHistory(Type page)
    {
        var stack = ContentFrame.BackStack;
        for (var index = stack.Count - 1; index >= 0; index--)
        {
            if (stack[index].SourcePageType == page)
            {
                stack.RemoveAt(index);
            }
        }
        AppTitleBar.IsBackButtonEnabled = ContentFrame.CanGoBack;
    }

    private async void OnRestartEngine(object sender, RoutedEventArgs e)
    {
        ShowStarting();
        await AppHost.StartEngineAsync(DispatcherQueue);
    }

    private void OnNavigationItemInvoked(NavigationView sender, NavigationViewItemInvokedEventArgs args)
    {
        if (args.InvokedItemContainer == AccountItem)
        {
            FlyoutBase.ShowAttachedFlyout(AccountItem);
        }
    }

    private async void OnSignOut(object sender, RoutedEventArgs e)
    {
        FlyoutBase.GetAttachedFlyout(AccountItem)?.Hide();
        var message = _running > 0
            ? $"还有 {_running} 个下载任务未完成，它们会在下次登录后继续。保存在本机的登录状态会被删除。"
            : "保存在本机的登录状态会被删除，下次使用需要重新扫码。";
        if (Content.XamlRoot is not { } root || !await Dialogs.ConfirmAsync(root, "退出 Canvas 登录？", message, "退出登录"))
        {
            return;
        }
        try
        {
            await AppHost.SignOutAsync();
        }
        catch (EngineException error)
        {
            ShowError("无法退出登录", error.Message);
        }
    }

    private void OnDownloadChanged(DownloadInfo info)
    {
        if (info.Status == "completed")
        {
            _completedInBatch++;
        }
        else if (info.Status == "failed")
        {
            _failedInBatch++;
        }
        _countsTimer.Stop();
        _countsTimer.Start();
    }

    private void OnDownloadRemoved(string id)
    {
        _countsTimer.Stop();
        _countsTimer.Start();
    }

    private async Task RefreshCountsAsync()
    {
        try
        {
            var counts = (await AppHost.RequireEngine().CallAsync<DownloadList>("downloads.list", new { limit = 1 })).Counts;
            _running = counts.Running;
            RunningBadge.Value = (int)Math.Min(counts.Running, 99);
            RunningBadge.Visibility = counts.Running > 0 ? Visibility.Visible : Visibility.Collapsed;
            if (counts.Running > 0)
            {
                _wasRunning = true;
            }
            else if (_wasRunning)
            {
                _wasRunning = false;
                NotifyBatchFinished();
            }
        }
        catch (EngineException)
        {
            RunningBadge.Visibility = Visibility.Collapsed;
        }
    }

    /// <summary>A toast when the queue empties while the app is in the background.</summary>
    private void NotifyBatchFinished()
    {
        var (completed, failed) = (_completedInBatch, _failedInBatch);
        _completedInBatch = _failedInBatch = 0;
        if (!_notificationsRegistered || completed + failed == 0 || GetForegroundWindow() == App.WindowHandle)
        {
            return;
        }
        try
        {
            var text = failed > 0 ? $"已完成 {completed} 个，{failed} 个失败" : $"已完成 {completed} 个文件";
            var builder = new AppNotificationBuilder()
                .AddArgument("open", "downloads")
                .AddText(failed > 0 ? "下载结束" : "下载完成")
                .AddText(text);
            AppNotificationManager.Default.Show(builder.BuildNotification());
        }
        catch (Exception)
        {
            // Notifications are best-effort.
        }
    }

    private void RegisterNotifications()
    {
        if (AppHost.IsUiTest)
        {
            return;
        }
        try
        {
            AppNotificationManager.Default.NotificationInvoked += (_, _) =>
            {
                DispatcherQueue.TryEnqueue(() =>
                {
                    BringToFront();
                    if (AppHost.IsSignedIn)
                    {
                        Navigate(typeof(DownloadsPage));
                    }
                });
            };
            AppNotificationManager.Default.Register();
            _notificationsRegistered = true;
        }
        catch (Exception)
        {
            _notificationsRegistered = false;
        }
    }

    private void OnNavigationSelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        if (args.IsSettingsSelected)
        {
            Navigate(typeof(SettingsPage));
        }
        else if (args.SelectedItem is NavigationViewItem { Tag: string tag })
        {
            Navigate(tag == "downloads" ? typeof(DownloadsPage) : typeof(CoursesPage));
        }
    }

    private void OnNavigated(object sender, NavigationEventArgs e)
    {
        AppTitleBar.IsBackButtonEnabled = ContentFrame.CanGoBack;
        object? selected = e.SourcePageType == typeof(SettingsPage) ? NavView.SettingsItem
            : e.SourcePageType == typeof(DownloadsPage) ? DownloadsItem
            : e.SourcePageType == typeof(CoursesPage) || e.SourcePageType == typeof(CoursePage) ? CoursesItem
            : null;
        if (!ReferenceEquals(NavView.SelectedItem, selected))
        {
            NavView.SelectionChanged -= OnNavigationSelectionChanged;
            NavView.SelectedItem = selected;
            NavView.SelectionChanged += OnNavigationSelectionChanged;
        }
    }

    private void OnBackRequested(TitleBar sender, object args)
    {
        if (ContentFrame.CanGoBack)
        {
            ContentFrame.GoBack();
        }
    }

    private void OnPaneToggleRequested(TitleBar sender, object args)
    {
        NavView.IsPaneOpen = !NavView.IsPaneOpen;
    }

    private void RestorePlacement()
    {
        var host = AppHost.Host;
        var scale = GetDpiForWindow(App.WindowHandleFor(this)) / 96.0;
        AppWindow.Resize(new SizeInt32((int)(host.WindowWidth * scale), (int)(host.WindowHeight * scale)));
        if (AppWindow.Presenter is OverlappedPresenter presenter)
        {
            presenter.PreferredMinimumWidth = (int)(760 * scale);
            presenter.PreferredMinimumHeight = (int)(560 * scale);
            if (host.WindowMaximized)
            {
                presenter.Maximize();
            }
        }
    }

    /// <summary>
    /// Asks before quitting with unfinished downloads, then stops the engine
    /// without blocking the UI thread; the downloads resume on the next launch.
    /// </summary>
    private async void OnClosing(AppWindow sender, AppWindowClosingEventArgs args)
    {
        if (_closing)
        {
            return;
        }
        args.Cancel = true;
        if (_running > 0 && Content.XamlRoot is { } root)
        {
            var quit = await Dialogs.ConfirmAsync(
                root,
                "退出 SJTU Canvas Downloader？",
                $"还有 {_running} 个下载任务未完成。退出后它们会暂停，下次打开应用时从断点继续。",
                "退出");
            if (!quit)
            {
                return;
            }
        }
        _closing = true;
        SavePlacement();
        AppWindow.Hide();
        try
        {
            await AppHost.StopEngineAsync();
        }
        catch (Exception error)
        {
            AppLog.Error("engine shutdown", error);
        }
        Close();
    }

    private void SavePlacement()
    {
        if (AppHost.IsUiTest)
        {
            return;
        }
        var host = AppHost.Host;
        if (AppWindow.Presenter is OverlappedPresenter presenter)
        {
            host.WindowMaximized = presenter.State == OverlappedPresenterState.Maximized;
            if (presenter.State == OverlappedPresenterState.Restored)
            {
                var scale = GetDpiForWindow(App.WindowHandleFor(this)) / 96.0;
                host.WindowWidth = (int)(AppWindow.Size.Width / scale);
                host.WindowHeight = (int)(AppWindow.Size.Height / scale);
            }
        }
        host.Save();
    }

    private void OnClosed(object sender, WindowEventArgs args)
    {
        if (_notificationsRegistered)
        {
            try
            {
                AppNotificationManager.Default.Unregister();
            }
            catch (Exception)
            {
                // Ignore; the process is exiting.
            }
        }
    }

    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetForegroundWindow(IntPtr hwnd);
}
