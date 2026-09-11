namespace CanvasDownloader.Services;

/// <summary>
/// Finds the engine binary. Installed layout: <c>engine\sjtu-canvas-engine.exe</c>
/// next to SJTUCanvasDownloader.exe. During development the repository build
/// output is used, or SJTU_CANVAS_ENGINE.
/// </summary>
public static class EngineLocator
{
    private const string EngineFile = "sjtu-canvas-engine.exe";

    public static string Locate()
    {
        var baseDir = AppContext.BaseDirectory;
        // A debug app uses the debug engine (only it has the test mode).
#if DEBUG
        string[] profiles = ["debug", "release"];
#else
        string[] profiles = ["release", "debug"];
#endif
        var engine = FirstExisting(
            Environment.GetEnvironmentVariable("SJTU_CANVAS_ENGINE"),
            Path.Combine(baseDir, "engine", EngineFile),
            Path.Combine(baseDir, EngineFile),
            RepoPath(baseDir, "engine", "target", profiles[0], EngineFile),
            RepoPath(baseDir, "engine", "target", profiles[1], EngineFile));
        return engine ?? throw new FileNotFoundException($"找不到下载引擎 {EngineFile}，请重新安装应用。");
    }

    private static string? FirstExisting(params string?[] candidates) =>
        candidates.FirstOrDefault(path => !string.IsNullOrEmpty(path) && File.Exists(path));

    /// <summary>Walks up from the app folder to a repository checkout.</summary>
    private static string? RepoPath(string start, params string[] parts)
    {
        var directory = new DirectoryInfo(start);
        while (directory is not null)
        {
            if (File.Exists(Path.Combine(directory.FullName, "engine", "Cargo.toml")))
            {
                return Path.Combine([directory.FullName, .. parts]);
            }
            directory = directory.Parent;
        }
        return null;
    }
}
