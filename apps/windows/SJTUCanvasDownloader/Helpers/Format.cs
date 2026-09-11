using System.Globalization;
using Microsoft.UI.Xaml;

namespace CanvasDownloader.Helpers;

/// <summary>Display text shared by the views (also used from x:Bind).</summary>
public static class Format
{
    private static readonly CultureInfo Chinese = CultureInfo.GetCultureInfo("zh-CN");
    private static readonly string[] Weekdays = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

    public static string Size(long bytes) => bytes switch
    {
        < 1024 => $"{bytes} B",
        < 1024 * 1024 => $"{bytes / 1024.0:0} KB",
        < 1024L * 1024 * 1024 => $"{bytes / 1024.0 / 1024.0:0.0} MB",
        _ => $"{bytes / 1024.0 / 1024.0 / 1024.0:0.00} GB",
    };

    public static string Speed(double bytesPerSecond) => bytesPerSecond <= 0 ? "" : $"{Size((long)bytesPerSecond)}/s";

    /// <summary>"剩余 3 分钟" while the speed is known.</summary>
    public static string Remaining(long received, long? total, double speed)
    {
        if (total is not { } whole || speed < 1 || whole <= received)
        {
            return "";
        }
        var seconds = (whole - received) / speed;
        return seconds switch
        {
            < 60 => $"剩余 {Math.Max(1, (int)seconds)} 秒",
            < 3600 => $"剩余 {(int)Math.Ceiling(seconds / 60)} 分钟",
            _ => $"剩余 {seconds / 3600:0.0} 小时",
        };
    }

    /// <summary>"9月1日 周二 08:00–09:40" from the school's local times.</summary>
    public static string LessonTime(string begin, string end)
    {
        if (!TryParseSchoolTime(begin, out var start))
        {
            return begin;
        }
        var text = $"{start.Month}月{start.Day}日 {Weekdays[(int)start.DayOfWeek]} {start:HH:mm}";
        if (TryParseSchoolTime(end, out var finish) && finish.Date == start.Date)
        {
            text += $"–{finish:HH:mm}";
        }
        return start.Year == DateTime.Now.Year ? text : $"{start.Year}年{text}";
    }

    private static bool TryParseSchoolTime(string value, out DateTime time) =>
        DateTime.TryParseExact(
            value.Trim(),
            ["yyyy-MM-dd HH:mm:ss", "yyyy-MM-dd HH:mm", "yyyy-MM-ddTHH:mm:ss", "yyyy-MM-ddTHH:mm"],
            CultureInfo.InvariantCulture,
            DateTimeStyles.None,
            out time);

    public static string Date(DateTimeOffset value) =>
        value.ToLocalTime().ToString("yyyy-MM-dd HH:mm", Chinese);

    public static string RelativeDate(DateTimeOffset value)
    {
        var local = value.ToLocalTime();
        var now = DateTimeOffset.Now;
        if (local.Date == now.Date)
        {
            return "今天 " + local.ToString("HH:mm", Chinese);
        }
        if (local.Date == now.Date.AddDays(-1))
        {
            return "昨天 " + local.ToString("HH:mm", Chinese);
        }
        return local.Year == now.Year
            ? local.ToString("M月d日 HH:mm", Chinese)
            : local.ToString("yyyy年M月d日", Chinese);
    }

    public static string TrackLabel(string? track) => track switch
    {
        "slides" => "电脑屏幕",
        "teacher" => "教室摄像头",
        "composite" => "合成画面",
        _ => "视频",
    };

    public static string DownloadStatus(string status) => status switch
    {
        "queued" => "排队中",
        "downloading" => "下载中",
        "paused" => "已暂停",
        "completed" => "已完成",
        "failed" => "失败",
        "cancelled" => "已取消",
        _ => status,
    };

    /// <summary>Segoe Fluent Icons glyph for a Canvas file.</summary>
    public static string FileGlyph(string? contentType, string name)
    {
        var extension = Path.GetExtension(name).ToLowerInvariant();
        return (contentType ?? "", extension) switch
        {
            (_, ".pdf") => "\uEA90",
            (_, ".zip" or ".rar" or ".7z") => "\uF012",
            (var type, _) when type.StartsWith("video/", StringComparison.Ordinal) => "\uE714",
            (var type, _) when type.StartsWith("image/", StringComparison.Ordinal) => "\uEB9F",
            _ => "\uE8A5",
        };
    }

    public static Visibility Show(bool value) => value ? Visibility.Visible : Visibility.Collapsed;

    public static Visibility Hide(bool value) => value ? Visibility.Collapsed : Visibility.Visible;

    public static bool Not(bool value) => !value;

    public static bool HasText(string? value) => !string.IsNullOrWhiteSpace(value);

    public static Visibility ShowText(string? value) =>
        string.IsNullOrWhiteSpace(value) ? Visibility.Collapsed : Visibility.Visible;
}
