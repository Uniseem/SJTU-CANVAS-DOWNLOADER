using CommunityToolkit.Mvvm.ComponentModel;
using CanvasDownloader.Services;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;

namespace CanvasDownloader.ViewModels;

/// <summary>The QR login: the image the engine pushes and the status text.</summary>
public sealed partial class LoginViewModel : ObservableObject
{
    private EngineClient? _engine;
    private string _attempt = "";
    private long _revision = -1;
    private int _generation = -1;

    [ObservableProperty]
    public partial ImageSource? QrImage { get; set; }

    [ObservableProperty]
    public partial string Message { get; set; } = "正在准备扫码登录…";

    [ObservableProperty]
    public partial bool IsBusy { get; set; } = true;

    [ObservableProperty]
    public partial bool IsWaiting { get; set; }

    [ObservableProperty]
    public partial bool IsExpired { get; set; }

    [ObservableProperty]
    public partial bool IsFailed { get; set; }

    [ObservableProperty]
    public partial bool IsDone { get; set; }

    public void Attach()
    {
        Detach();
        _engine = AppHost.Engine;
        if (_engine is not null)
        {
            _engine.LoginStatusChanged += Apply;
        }
    }

    public void Detach()
    {
        if (_engine is not null)
        {
            _engine.LoginStatusChanged -= Apply;
            _engine = null;
        }
    }

    public async Task StartAsync()
    {
        try
        {
            Apply(await AppHost.RequireEngine().CallAsync<LoginStatus>("login.start"));
        }
        catch (EngineException error)
        {
            ShowFailure(error.Message);
        }
    }

    public async Task RefreshAsync()
    {
        IsBusy = true;
        IsFailed = false;
        Message = "正在获取新的二维码…";
        try
        {
            await AppHost.RequireEngine().CallAsync("login.refresh");
        }
        catch (EngineException error)
        {
            ShowFailure(error.Message);
        }
    }

    public async Task CancelAsync()
    {
        try
        {
            await AppHost.RequireEngine().CallAsync("login.cancel");
        }
        catch (EngineException)
        {
            // Nothing to cancel.
        }
    }

    private void ShowFailure(string message)
    {
        IsBusy = false;
        IsWaiting = false;
        IsExpired = false;
        IsFailed = true;
        Message = message;
    }

    private async void Apply(LoginStatus status)
    {
        // Pushed and returned states can arrive out of order.
        if (status.AttemptId == _attempt && (status.Generation < _generation || status.Revision <= _revision))
        {
            return;
        }
        _attempt = status.AttemptId;
        _revision = status.Revision;
        _generation = status.Generation;
        Message = status.Message;
        IsBusy = status.State is "preparing" or "reconnecting" or "authorizing";
        IsWaiting = status.State == "waiting";
        IsExpired = status.State == "expired";
        IsFailed = status.State is "error" or "cancelled";
        IsDone = status.State == "authorized";
        if (status.State == "waiting" && status.QrPng is { Length: > 0 } png)
        {
            QrImage = await DecodeAsync(png);
        }
        else if (!IsExpired)
        {
            QrImage = null;
        }
    }

    private static async Task<ImageSource?> DecodeAsync(string base64)
    {
        try
        {
            var image = new BitmapImage();
            using var stream = new MemoryStream(Convert.FromBase64String(base64));
            await image.SetSourceAsync(stream.AsRandomAccessStream());
            return image;
        }
        catch (Exception error)
        {
            AppLog.Error("qr image", error);
            return null;
        }
    }
}
