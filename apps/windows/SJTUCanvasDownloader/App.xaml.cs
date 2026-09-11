using CanvasDownloader.Services;
using Microsoft.UI.Xaml;
using WinRT.Interop;

namespace CanvasDownloader;

public partial class App : Application
{
    private static MainWindow? _window;

    public App()
    {
        InitializeComponent();
        UnhandledException += (_, args) =>
        {
            args.Handled = true;
            AppLog.Error("unhandled", args.Exception);
            _window?.ShowError("发生意外错误", args.Exception.Message);
        };
    }

    public static MainWindow Window => _window ?? throw new InvalidOperationException("窗口尚未创建");

    /// <summary>
    /// Called first thing in the MainWindow constructor: its initial
    /// navigation already needs <see cref="Window"/>.
    /// </summary>
    internal static void Attach(MainWindow window) => _window = window;

    public static IntPtr WindowHandle => WindowNative.GetWindowHandle(Window);

    public static IntPtr WindowHandleFor(Microsoft.UI.Xaml.Window window) => WindowNative.GetWindowHandle(window);

    public static void BringToFront()
    {
        _window?.DispatcherQueue.TryEnqueue(() => _window.BringToFront());
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        _window = new MainWindow();
        if (AppHost.IsUiTest)
        {
            // Shown without activation and behind other windows, so a test
            // neither takes the user's focus nor covers what they work on.
            _window.AppWindow.Show(false);
            SetWindowPos(WindowHandle, new IntPtr(1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010);
        }
        else
        {
            _window.Activate();
        }
        _ = AppHost.StartEngineAsync(_window.DispatcherQueue);
    }

    // HWND_BOTTOM = 1; SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE.
    [System.Runtime.InteropServices.DllImport("user32.dll")]
    private static extern bool SetWindowPos(IntPtr hwnd, IntPtr insertAfter, int x, int y, int width, int height, uint flags);
}
