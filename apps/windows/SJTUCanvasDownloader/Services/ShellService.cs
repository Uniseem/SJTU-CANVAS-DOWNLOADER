using System.Diagnostics;
using System.Runtime.InteropServices;
using Windows.Storage.Pickers;
using WinRT.Interop;

namespace CanvasDownloader.Services;

/// <summary>Explorer integration, the Downloads folder and the folder picker.</summary>
public static partial class ShellService
{
    private static readonly Guid DownloadsFolderId = new("374DE290-123F-4565-9164-39C4925E467B");

    /// <summary>
    /// The user's Downloads folder, even when it was moved to another drive.
    /// SJTU_CANVAS_DOWNLOADS_FOLDER replaces it in tests.
    /// </summary>
    public static string DownloadsFolder
    {
        get
        {
            if (Environment.GetEnvironmentVariable("SJTU_CANVAS_DOWNLOADS_FOLDER") is { Length: > 0 } overridden)
            {
                return overridden;
            }
            if (SHGetKnownFolderPath(DownloadsFolderId, 0, IntPtr.Zero, out var pointer) == 0)
            {
                try
                {
                    if (Marshal.PtrToStringUni(pointer) is { Length: > 0 } path)
                    {
                        return path;
                    }
                }
                finally
                {
                    Marshal.FreeCoTaskMem(pointer);
                }
            }
            return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), "Downloads");
        }
    }

    public static void Open(string path)
    {
        Process.Start(new ProcessStartInfo(path) { UseShellExecute = true });
    }

    public static void OpenUri(Uri uri)
    {
        Process.Start(new ProcessStartInfo(uri.AbsoluteUri) { UseShellExecute = true });
    }

    /// <summary>Opens Explorer with the file selected, or the folder itself.</summary>
    public static void Reveal(string path)
    {
        if (File.Exists(path))
        {
            Process.Start(new ProcessStartInfo("explorer.exe") { ArgumentList = { "/select,", path } });
        }
        else if (Directory.Exists(path))
        {
            Process.Start(new ProcessStartInfo("explorer.exe") { ArgumentList = { path } });
        }
        else if (Path.GetDirectoryName(path) is { } parent && Directory.Exists(parent))
        {
            Process.Start(new ProcessStartInfo("explorer.exe") { ArgumentList = { parent } });
        }
    }

    public static async Task<string?> PickFolderAsync()
    {
        var picker = new FolderPicker { SuggestedStartLocation = PickerLocationId.Downloads };
        picker.FileTypeFilter.Add("*");
        InitializeWithWindow.Initialize(picker, App.WindowHandle);
        var folder = await picker.PickSingleFolderAsync();
        return folder?.Path;
    }

    [LibraryImport("shell32.dll")]
    private static partial int SHGetKnownFolderPath(in Guid id, uint flags, IntPtr token, out IntPtr path);
}
