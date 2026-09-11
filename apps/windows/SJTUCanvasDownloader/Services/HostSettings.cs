using System.Text.Json;

namespace CanvasDownloader.Services;

/// <summary>
/// Settings owned by the Windows app itself (not the engine): the window
/// placement. Stored in %LOCALAPPDATA%\SJTU Canvas Downloader\host.json.
/// </summary>
public sealed class HostSettings
{
    public static string AppDataDir { get; } =
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "SJTU Canvas Downloader");

    private static string FilePath => Path.Combine(AppDataDir, "host.json");

    public int WindowWidth { get; set; } = 1180;
    public int WindowHeight { get; set; } = 800;
    public bool WindowMaximized { get; set; }

    public static HostSettings Load()
    {
        try
        {
            if (File.Exists(FilePath))
            {
                return JsonSerializer.Deserialize<HostSettings>(File.ReadAllText(FilePath), EngineClient.Json) ?? new();
            }
        }
        catch (Exception)
        {
            // A damaged file falls back to defaults.
        }
        return new();
    }

    public void Save()
    {
        try
        {
            Directory.CreateDirectory(AppDataDir);
            var temporary = FilePath + ".tmp";
            File.WriteAllText(temporary, JsonSerializer.Serialize(this, EngineClient.Json));
            File.Move(temporary, FilePath, overwrite: true);
        }
        catch (Exception)
        {
            // Preferences are best-effort.
        }
    }
}
