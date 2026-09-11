using System.Text.Json.Serialization;

namespace CanvasDownloader.Services;

// Wire types of the sjtu-canvas-engine JSON-RPC protocol (engine/src/rpc.rs).
// Property names map to snake_case through the client's naming policy.

public sealed class Profile
{
    public string Id { get; set; } = "";
    public string Name { get; set; } = "";
    public string ShortName { get; set; } = "";
    public string? AvatarUrl { get; set; }
}

public sealed class AccountInfo
{
    public bool Authenticated { get; set; }
    public Profile? Profile { get; set; }
    /// <summary>False when the login could not be protected with a Credential Manager key.</summary>
    public bool Persisted { get; set; }
    public bool Verified { get; set; }
}

public sealed class LoginStatus
{
    public string AttemptId { get; set; } = "";
    /// <summary>preparing | waiting | reconnecting | authorizing | authorized | expired | cancelled | error</summary>
    public string State { get; set; } = "";
    public string Message { get; set; } = "";
    public int Generation { get; set; }
    public long Revision { get; set; }
    /// <summary>The QR code as a base64 PNG while waiting for the scan.</summary>
    public string? QrPng { get; set; }
    public DateTimeOffset? ExpiresAt { get; set; }
}

public sealed class Course
{
    public string Id { get; set; } = "";
    public string Name { get; set; } = "";
    public string CourseCode { get; set; } = "";
    public string? Term { get; set; }
    public string? Teacher { get; set; }
    /// <summary>active | invited_or_pending | completed</summary>
    public string EnrollmentState { get; set; } = "";
}

public sealed class CourseList
{
    public List<Course> Courses { get; set; } = [];
}

public sealed class Lesson
{
    public string VideoId { get; set; } = "";
    public string Title { get; set; } = "";
    public string BeginTime { get; set; } = "";
    public string EndTime { get; set; } = "";
    public string Classroom { get; set; } = "";
    public bool Available { get; set; }
    /// <summary>resource | canvas-lti | historical (课堂视频旧版)</summary>
    public string? Source { get; set; }
}

public sealed class LessonList
{
    public List<Lesson> Lessons { get; set; } = [];
}

public sealed class CourseFile
{
    public string Id { get; set; } = "";
    public string DisplayName { get; set; } = "";
    public string Filename { get; set; } = "";
    public long Size { get; set; }
    public string? ContentType { get; set; }
    public DateTimeOffset? UpdatedAt { get; set; }
}

public sealed class FileList
{
    public List<CourseFile> Files { get; set; } = [];
}

public sealed class TrackSize
{
    /// <summary>ready | missing | unavailable</summary>
    public string Status { get; set; } = "";
    public long? Size { get; set; }
}

public sealed class LessonSizes
{
    public string VideoId { get; set; } = "";
    public Dictionary<string, TrackSize> Tracks { get; set; } = [];
}

public sealed class DownloadInfo
{
    public string Id { get; set; } = "";
    /// <summary>video | file</summary>
    public string Kind { get; set; } = "";
    public string CourseId { get; set; } = "";
    public string CourseName { get; set; } = "";
    public string ResourceId { get; set; } = "";
    public string? Track { get; set; }
    public string Title { get; set; } = "";
    public string DisplayName { get; set; } = "";
    public string? BeginTime { get; set; }
    public string Destination { get; set; } = "";
    public string? FilePath { get; set; }
    /// <summary>queued | downloading | paused | completed | failed | cancelled</summary>
    public string Status { get; set; } = "";
    public long Received { get; set; }
    public long? Total { get; set; }
    public double Speed { get; set; }
    public string? Error { get; set; }
    public DateTimeOffset CreatedAt { get; set; }
    public DateTimeOffset? CompletedAt { get; set; }

    [JsonIgnore]
    public bool IsUnfinished => Status is "queued" or "downloading" or "paused";

    [JsonIgnore]
    public bool IsStopped => Status is "failed" or "cancelled";
}

public sealed class DownloadProgress
{
    public string Id { get; set; } = "";
    public long Received { get; set; }
    public long? Total { get; set; }
    public double Speed { get; set; }
}

public sealed class DownloadCounts
{
    public long All { get; set; }
    public long Active { get; set; }
    public long Running { get; set; }
    public long Completed { get; set; }
    public long Failed { get; set; }
}

public sealed class DownloadList
{
    public List<DownloadInfo> Items { get; set; } = [];
    public DownloadCounts Counts { get; set; } = new();
}

public sealed class CreateResult
{
    public List<DownloadInfo> Created { get; set; } = [];
    public List<SkippedItem> Skipped { get; set; } = [];
}

public sealed class SkippedItem
{
    public int Index { get; set; }

    /// <summary>invalid | queued | downloaded</summary>
    public string Reason { get; set; } = "";

    public string Message { get; set; } = "";
}

/// <summary>One item of downloads.create; unused fields must stay off the wire.</summary>
public sealed class NewDownload
{
    public string Kind { get; set; } = "video";
    public string CourseId { get; set; } = "";
    public string CourseName { get; set; } = "";

    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? LessonId { get; set; }

    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? FileId { get; set; }

    public string Title { get; set; } = "";

    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? BeginTime { get; set; }

    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? Track { get; set; }

    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public long? Size { get; set; }
}

public sealed class ProxySettings
{
    /// <summary>system | direct | custom</summary>
    public string Mode { get; set; } = "system";

    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? Url { get; set; }
}

public sealed class Preferences
{
    public string DownloadDir { get; set; } = "";
    public bool AskDestination { get; set; }
    public int Concurrency { get; set; } = 3;
    public List<string> DefaultTracks { get; set; } = ["slides", "teacher"];
    public ProxySettings Proxy { get; set; } = new();

    public Preferences Clone() => new()
    {
        DownloadDir = DownloadDir,
        AskDestination = AskDestination,
        Concurrency = Concurrency,
        DefaultTracks = [.. DefaultTracks],
        Proxy = new ProxySettings { Mode = Proxy.Mode, Url = Proxy.Url },
    };
}

public sealed class SettingsInfo
{
    public Preferences Preferences { get; set; } = new();
    public string DefaultDownloadDir { get; set; } = "";
    public int ConcurrencyMax { get; set; } = 8;
    public string DataDir { get; set; } = "";
    public string EngineVersion { get; set; } = "";
    public bool TestMode { get; set; }
    public bool FakeSchool { get; set; }
}

public sealed class InitializeResult
{
    public int Protocol { get; set; }
    public string Version { get; set; } = "";
    public string DataDir { get; set; } = "";
    public SettingsInfo Settings { get; set; } = new();
    public AccountInfo Account { get; set; } = new();
}
