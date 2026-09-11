using Microsoft.UI.Dispatching;

namespace CanvasDownloader.Services;

/// <summary>
/// Process-wide services: the engine connection, the account, the latest
/// engine settings and the app's own preferences.
/// </summary>
public static class AppHost
{
    public static HostSettings Host { get; } = HostSettings.Load();

    public static EngineClient? Engine { get; private set; }

    public static SettingsInfo? Settings { get; private set; }

    public static AccountInfo Account { get; private set; } = new();

    public static string? EngineVersion { get; private set; }

    public static string? StartupError { get; private set; }

    /// <summary>The data folder; SJTU_CANVAS_DATA_DIR overrides it for development.</summary>
    public static string DataDir =>
        Environment.GetEnvironmentVariable("SJTU_CANVAS_DATA_DIR") is { Length: > 0 } overridden
            ? overridden
            : HostSettings.AppDataDir;

    /// <summary>
    /// UI tests (SJTU_CANVAS_UI_TEST=1): the window opens without taking the
    /// focus, and nothing is registered with Windows (notifications, window
    /// placement).
    /// </summary>
    public static bool IsUiTest => Environment.GetEnvironmentVariable("SJTU_CANVAS_UI_TEST") == "1";

    /// <summary>The key protecting the saved login; SJTU_CANVAS_SESSION_KEY replaces Credential Manager in tests.</summary>
    private static string? SessionKey =>
        Environment.GetEnvironmentVariable("SJTU_CANVAS_SESSION_KEY") is { Length: > 0 } key
            ? key
            : CredentialStore.GetOrCreateSessionKey();

    /// <summary>Raised on the UI thread when the engine starts or stops.</summary>
    public static event Action? EngineStateChanged;

    /// <summary>Raised on the UI thread after the engine settings change.</summary>
    public static event Action? SettingsChanged;

    /// <summary>Raised on the UI thread when the login starts or ends.</summary>
    public static event Action? AccountChanged;

    public static bool IsReady => Engine?.IsRunning == true && Settings is not null;

    public static bool IsSignedIn => IsReady && Account.Authenticated;

    public static EngineClient RequireEngine() =>
        Engine is { IsRunning: true } engine ? engine : throw new EngineException("engine_stopped", "下载引擎未运行");

    public static async Task StartEngineAsync(DispatcherQueue dispatcher)
    {
        StartupError = null;
        if (Engine is not null)
        {
            await Engine.DisposeAsync();
            Engine = null;
        }

        try
        {
            var enginePath = EngineLocator.Locate();
            Directory.CreateDirectory(DataDir);
            var engine = new EngineClient(dispatcher);
            engine.Exited += message =>
            {
                StartupError = message;
                EngineStateChanged?.Invoke();
            };
            engine.AccountChanged += account =>
            {
                Account = account;
                AccountChanged?.Invoke();
            };
            engine.Start(enginePath, DataDir);
            var result = await engine.CallAsync<InitializeResult>("engine.initialize", new
            {
                session_key = SessionKey,
                downloads_folder = ShellService.DownloadsFolder,
            });
            Engine = engine;
            EngineVersion = result.Version;
            Settings = result.Settings;
            Account = result.Account;
        }
        catch (Exception error)
        {
            StartupError = error.Message;
        }
        EngineStateChanged?.Invoke();
        SettingsChanged?.Invoke();
        AccountChanged?.Invoke();
        if (Engine is { } running && Account.Authenticated)
        {
            _ = VerifyAccountAsync(running);
        }
    }

    /// <summary>Confirms a restored login with Canvas; an expired one ends here.</summary>
    private static async Task VerifyAccountAsync(EngineClient engine)
    {
        try
        {
            var account = await engine.CallAsync<AccountInfo>("account.get", new { verify = true });
            if (account.Authenticated != Account.Authenticated || account.Profile?.Name != Account.Profile?.Name)
            {
                Account = account;
                AccountChanged?.Invoke();
            }
        }
        catch (EngineException)
        {
            // Offline: keep the saved login; requests report their own errors.
        }
    }

    public static async Task SignOutAsync()
    {
        Account = await RequireEngine().CallAsync<AccountInfo>("account.logout");
        AccountChanged?.Invoke();
    }

    public static void UpdateSettings(SettingsInfo settings)
    {
        Settings = settings;
        SettingsChanged?.Invoke();
    }

    public static async Task StopEngineAsync()
    {
        if (Engine is { } engine)
        {
            Engine = null;
            await engine.DisposeAsync();
        }
    }
}
