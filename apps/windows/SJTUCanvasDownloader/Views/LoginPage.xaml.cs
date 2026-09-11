using CanvasDownloader.Services;
using CanvasDownloader.ViewModels;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;

namespace CanvasDownloader.Views;

public sealed partial class LoginPage : Page
{
    public LoginPage()
    {
        InitializeComponent();
    }

    public LoginViewModel ViewModel { get; } = new();

    protected override async void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        if (AppHost.IsSignedIn)
        {
            // Already signed in (a stale history entry): never start a new login.
            DispatcherQueue.TryEnqueue(() => App.Window.Navigate(typeof(CoursesPage), clearHistory: true));
            return;
        }
        AppHost.EngineStateChanged += OnEngineStateChanged;
        ViewModel.Attach();
        if (AppHost.IsReady)
        {
            await ViewModel.StartAsync();
        }
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
        if (AppHost.IsReady && !AppHost.Account.Authenticated)
        {
            await ViewModel.StartAsync();
        }
    }

    private async void OnRefresh(object sender, RoutedEventArgs e) => await ViewModel.RefreshAsync();

    private void OnOpenSettings(object sender, RoutedEventArgs e) => App.Window.Navigate(typeof(SettingsPage));

    public Style RefreshStyle(bool failed) =>
        (Style)Application.Current.Resources[failed ? "AccentButtonStyle" : "DefaultButtonStyle"];
}
