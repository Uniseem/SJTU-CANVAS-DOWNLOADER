namespace CanvasDownloader.Services;

/// <summary>Append-only log of app errors, next to the engine log.</summary>
public static class AppLog
{
    private static readonly object Gate = new();

    public static void Error(string context, Exception error)
    {
        try
        {
            var directory = Path.Combine(AppHost.DataDir, "logs");
            Directory.CreateDirectory(directory);
            lock (Gate)
            {
                File.AppendAllText(
                    Path.Combine(directory, "app.log"),
                    $"{DateTimeOffset.Now:O} {context}: {error}{Environment.NewLine}");
            }
        }
        catch (Exception)
        {
            // Logging must never take the app down.
        }
    }
}
