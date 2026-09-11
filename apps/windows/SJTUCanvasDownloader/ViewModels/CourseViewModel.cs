using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using CanvasDownloader.Helpers;
using CanvasDownloader.Services;

namespace CanvasDownloader.ViewModels;

/// <summary>One lesson recording; its size is looked up once the row is shown.</summary>
public sealed partial class LessonItemViewModel(Lesson lesson) : ObservableObject
{
    public Lesson Lesson { get; } = lesson;

    public string Id => Lesson.VideoId;

    public string Title => Lesson.Title;

    public bool Available => Lesson.Available;

    public bool Unavailable => !Lesson.Available;

    public string DownloadLabel => $"下载 {Title}";

    public string Meta
    {
        get
        {
            var parts = new List<string> { Format.LessonTime(Lesson.BeginTime, Lesson.EndTime) };
            parts.Add(string.IsNullOrWhiteSpace(Lesson.Classroom) ? "教室未知" : Lesson.Classroom);
            if (Lesson.Source == "historical")
            {
                parts.Add("旧版录像");
            }
            return string.Join(" · ", parts.Where(part => part.Length > 0));
        }
    }

    [ObservableProperty]
    public partial string SizeText { get; set; } = "";

    [ObservableProperty]
    public partial string SizeTooltip { get; set; } = "";

    [ObservableProperty]
    public partial bool CanRetrySize { get; set; }

    /// <summary>The name screen readers and UI Automation read for the row.</summary>
    public override string ToString() => Title;
}

/// <summary>One Canvas course file.</summary>
public sealed class FileItemViewModel(CourseFile file)
{
    public CourseFile File { get; } = file;

    public string Id => File.Id;

    public string Name => string.IsNullOrWhiteSpace(File.DisplayName) ? File.Filename : File.DisplayName;

    /// <summary>The file name when it differs from the title, and when it was updated.</summary>
    public string Meta => string.Join(" · ", new[]
    {
        File.Filename == Name ? "" : File.Filename,
        File.UpdatedAt is { } updated ? $"更新于 {Format.RelativeDate(updated)}" : "",
    }.Where(part => part.Length > 0));

    public string SizeText => Format.Size(File.Size);

    public string Glyph => Format.FileGlyph(File.ContentType, File.Filename);

    public string DownloadLabel => $"下载 {Name}";

    public override string ToString() => Name;
}

/// <summary>A course: its lesson recordings and files, and the tracks to download.</summary>
public sealed partial class CourseViewModel : ObservableObject
{
    private static readonly string[] AllTracks = ["slides", "teacher", "composite"];
    private const int SizeConcurrency = 2;

    private readonly Dictionary<string, Dictionary<string, TrackSize>> _sizes = [];
    private readonly HashSet<string> _realized = [];
    private readonly HashSet<string> _fetching = [];
    private readonly List<(string Id, bool Refresh)> _queue = [];
    private List<LessonItemViewModel> _lessons = [];
    private List<FileItemViewModel> _files = [];
    private int _running;
    private bool _autoRetryUsed;
    private bool _disposed;
    private CancellationTokenSource? _countdown;

    public CourseViewModel(Course course)
    {
        Course = course;
        var defaults = AppHost.Settings?.Preferences.DefaultTracks ?? ["slides", "teacher"];
        Tracks = [.. AllTracks.Where(defaults.Contains)];
        if (Tracks.Count == 0)
        {
            Tracks = ["slides", "teacher"];
        }
    }

    public Course Course { get; }

    public string Title => Course.Name;

    public string Subtitle => string.Join(" · ", new[] { Course.CourseCode, Course.Term, Course.Teacher }.Where(value => !string.IsNullOrWhiteSpace(value)));

    /// <summary>Selected tracks in canonical order: slides, teacher, composite.</summary>
    public List<string> Tracks { get; private set; }

    public string TracksText => string.Join("、", Tracks.Select(Format.TrackLabel));

    public ObservableCollection<LessonItemViewModel> Lessons { get; } = [];

    public ObservableCollection<FileItemViewModel> Files { get; } = [];

    [ObservableProperty]
    public partial string Query { get; set; } = "";

    [ObservableProperty]
    public partial bool LessonsLoading { get; set; }

    [ObservableProperty]
    public partial bool FilesLoading { get; set; }

    [ObservableProperty]
    public partial string? LessonsError { get; set; }

    /// <summary>The lesson message is information (no recordings scheduled), not a failure.</summary>
    [ObservableProperty]
    public partial bool LessonsNotice { get; set; }

    [ObservableProperty]
    public partial string? FilesError { get; set; }

    [ObservableProperty]
    public partial int RetrySeconds { get; set; }

    [ObservableProperty]
    public partial int HistoricalCount { get; set; }

    [ObservableProperty]
    public partial int LessonCount { get; set; }

    [ObservableProperty]
    public partial int FileCount { get; set; }

    [ObservableProperty]
    public partial bool LessonsEmpty { get; set; }

    [ObservableProperty]
    public partial bool FilesEmpty { get; set; }

    public IEnumerable<LessonItemViewModel> AllLessons => _lessons;

    public async Task LoadAsync()
    {
        await Task.WhenAll(LoadLessonsAsync(), LoadFilesAsync());
    }

    public async Task LoadLessonsAsync(bool automatic = false)
    {
        CancelCountdown();
        if (!automatic)
        {
            _autoRetryUsed = false;
        }
        LessonsLoading = true;
        LessonsError = null;
        LessonsNotice = false;
        LessonsEmpty = false;
        try
        {
            var list = await AppHost.RequireEngine().CallAsync<LessonList>("courses.lessons", new { course_id = Course.Id });
            _lessons = list.Lessons.Select(lesson => new LessonItemViewModel(lesson)).ToList();
            HistoricalCount = list.Lessons.Count(lesson => lesson.Source == "historical");
            LessonCount = _lessons.Count;
            ApplyQuery();
            foreach (var lesson in _lessons)
            {
                UpdateSize(lesson);
            }
        }
        catch (EngineException error) when (!error.IsUnauthorized)
        {
            LessonsError = error.Message;
            LessonsNotice = error.Code == "video_unavailable";
            if (error.Code == "upstream_unavailable" && error.RetryAfterSeconds is > 0 and var seconds && !_autoRetryUsed)
            {
                StartCountdown(Math.Min(seconds, 120));
            }
        }
        catch (EngineException)
        {
            // Signed out: the window shows the login page.
        }
        finally
        {
            LessonsLoading = false;
            LessonsEmpty = LessonsError is null && Lessons.Count == 0;
        }
    }

    public async Task LoadFilesAsync()
    {
        FilesLoading = true;
        FilesError = null;
        FilesEmpty = false;
        try
        {
            var list = await AppHost.RequireEngine().CallAsync<FileList>("courses.files", new { course_id = Course.Id });
            _files = list.Files.Select(file => new FileItemViewModel(file)).ToList();
            FileCount = _files.Count;
            ApplyQuery();
        }
        catch (EngineException error) when (!error.IsUnauthorized)
        {
            FilesError = error.Message;
        }
        catch (EngineException)
        {
        }
        finally
        {
            FilesLoading = false;
            FilesEmpty = FilesError is null && Files.Count == 0;
        }
    }

    partial void OnQueryChanged(string value) => ApplyQuery();

    private void ApplyQuery()
    {
        var query = Query.Trim();
        bool Matches(string text) => query.Length == 0 || text.Contains(query, StringComparison.CurrentCultureIgnoreCase);
        Replace(Lessons, _lessons.Where(lesson => Matches($"{lesson.Title} {lesson.Lesson.Classroom}")));
        Replace(Files, _files.Where(file => Matches($"{file.Name} {file.File.Filename}")));
        LessonsEmpty = !LessonsLoading && LessonsError is null && Lessons.Count == 0;
        FilesEmpty = !FilesLoading && FilesError is null && Files.Count == 0;
    }

    private static void Replace<T>(ObservableCollection<T> target, IEnumerable<T> items)
    {
        target.Clear();
        foreach (var item in items)
        {
            target.Add(item);
        }
    }

    private void StartCountdown(int seconds)
    {
        _autoRetryUsed = true;
        var countdown = new CancellationTokenSource();
        _countdown = countdown;
        RetrySeconds = seconds;
        _ = RunCountdownAsync(countdown.Token);
    }

    private async Task RunCountdownAsync(CancellationToken cancellation)
    {
        try
        {
            while (RetrySeconds > 0)
            {
                await Task.Delay(1000, cancellation);
                RetrySeconds--;
            }
            await LoadLessonsAsync(automatic: true);
        }
        catch (TaskCanceledException)
        {
        }
    }

    private void CancelCountdown()
    {
        _countdown?.Cancel();
        _countdown = null;
        RetrySeconds = 0;
    }

    // Tracks

    public bool HasTrack(string track) => Tracks.Contains(track);

    /// <summary>Adds or removes a track; at least one stays selected.</summary>
    public void SetTrack(string track, bool selected)
    {
        var tracks = new HashSet<string>(Tracks);
        if (selected)
        {
            tracks.Add(track);
        }
        else if (tracks.Count > 1)
        {
            tracks.Remove(track);
        }
        Tracks = [.. AllTracks.Where(tracks.Contains)];
        OnPropertyChanged(nameof(TracksText));
        foreach (var lesson in _lessons)
        {
            UpdateSize(lesson);
        }
        foreach (var id in _realized)
        {
            Enqueue(id, refresh: false);
        }
        Pump();
    }

    // Sizes: looked up for rows on screen, two lessons at a time.

    public void SetRealized(LessonItemViewModel lesson, bool realized)
    {
        if (!lesson.Available)
        {
            return;
        }
        if (realized)
        {
            _realized.Add(lesson.Id);
            Enqueue(lesson.Id, refresh: false);
            Pump();
        }
        else
        {
            _realized.Remove(lesson.Id);
            _queue.RemoveAll(entry => entry.Id == lesson.Id && !entry.Refresh);
        }
    }

    public void RetrySize(LessonItemViewModel lesson)
    {
        if (_sizes.TryGetValue(lesson.Id, out var known))
        {
            foreach (var track in Tracks.Where(track => known.TryGetValue(track, out var size) && size.Status == "unavailable"))
            {
                known.Remove(track);
            }
        }
        Enqueue(lesson.Id, refresh: true);
        Pump();
    }

    private List<string> MissingTracks(string id) =>
        Tracks.Where(track => !_sizes.TryGetValue(id, out var known) || !known.ContainsKey(track)).ToList();

    private void Enqueue(string id, bool refresh)
    {
        if (_fetching.Contains(id) || _queue.Any(entry => entry.Id == id) || MissingTracks(id).Count == 0)
        {
            return;
        }
        _queue.Add((id, refresh));
        if (_lessons.FirstOrDefault(lesson => lesson.Id == id) is { } item)
        {
            UpdateSize(item);
        }
    }

    private void Pump()
    {
        while (!_disposed && _running < SizeConcurrency && _queue.Count > 0)
        {
            var (id, refresh) = _queue[0];
            _queue.RemoveAt(0);
            var missing = MissingTracks(id);
            if (missing.Count == 0)
            {
                continue;
            }
            _running++;
            _fetching.Add(id);
            _ = FetchSizesAsync(id, missing, refresh);
        }
    }

    private async Task FetchSizesAsync(string id, List<string> tracks, bool refresh)
    {
        Dictionary<string, TrackSize> result;
        try
        {
            var sizes = await AppHost.RequireEngine().CallAsync<LessonSizes>("lessons.sizes", new
            {
                course_id = Course.Id,
                lesson_id = id,
                tracks,
                refresh,
            });
            result = tracks.ToDictionary(track => track, track => sizes.Tracks.GetValueOrDefault(track) ?? new TrackSize { Status = "unavailable" });
        }
        catch (EngineException)
        {
            result = tracks.ToDictionary(track => track, _ => new TrackSize { Status = "unavailable" });
        }
        _running--;
        _fetching.Remove(id);
        if (_disposed)
        {
            return;
        }
        if (!_sizes.TryGetValue(id, out var known))
        {
            known = [];
            _sizes[id] = known;
        }
        foreach (var (track, size) in result)
        {
            known[track] = size;
        }
        if (_lessons.FirstOrDefault(lesson => lesson.Id == id) is { } item)
        {
            UpdateSize(item);
        }
        Pump();
    }

    private void UpdateSize(LessonItemViewModel lesson)
    {
        lesson.CanRetrySize = false;
        if (!lesson.Available)
        {
            lesson.SizeText = "";
            lesson.SizeTooltip = "";
            return;
        }
        _sizes.TryGetValue(lesson.Id, out var known);
        var values = Tracks.Select(track => known?.GetValueOrDefault(track)).ToList();
        lesson.SizeTooltip = string.Join(Environment.NewLine, Tracks.Select(track =>
        {
            var size = known?.GetValueOrDefault(track);
            var text = size?.Status switch
            {
                "ready" when size.Size is { } bytes => Format.Size(bytes),
                "missing" => "没有这个画面",
                "unavailable" => "暂时无法获取",
                _ => "尚未获取",
            };
            return $"{Format.TrackLabel(track)}：{text}";
        }));
        if (values.Any(value => value is null))
        {
            var pending = _fetching.Contains(lesson.Id) || _queue.Any(entry => entry.Id == lesson.Id);
            lesson.SizeText = pending ? "读取中…" : "—";
        }
        else if (values.Any(value => value!.Status == "missing"))
        {
            lesson.SizeText = "无所选画面";
        }
        else if (values.Any(value => value!.Status != "ready" || value.Size is null))
        {
            lesson.SizeText = "大小未知";
            lesson.CanRetrySize = true;
        }
        else
        {
            lesson.SizeText = Format.Size(values.Sum(value => value!.Size!.Value));
        }
    }

    // Downloads

    public List<NewDownload> DownloadsFor(IEnumerable<LessonItemViewModel> lessons) =>
        lessons.Where(lesson => lesson.Available)
            .SelectMany(lesson => Tracks.Select(track => new NewDownload
            {
                Kind = "video",
                CourseId = Course.Id,
                CourseName = Course.Name,
                LessonId = lesson.Id,
                Title = lesson.Title,
                BeginTime = string.IsNullOrWhiteSpace(lesson.Lesson.BeginTime) ? null : lesson.Lesson.BeginTime,
                Track = track,
                Size = _sizes.GetValueOrDefault(lesson.Id)?.GetValueOrDefault(track) is { Status: "ready", Size: { } size } ? size : null,
            }))
            .ToList();

    public List<NewDownload> DownloadsFor(IEnumerable<FileItemViewModel> files) =>
        files.Select(file => new NewDownload
        {
            Kind = "file",
            CourseId = Course.Id,
            CourseName = Course.Name,
            FileId = file.Id,
            Title = file.Name,
            Size = file.File.Size > 0 ? file.File.Size : null,
        }).ToList();

    public void Dispose()
    {
        _disposed = true;
        CancelCountdown();
        _queue.Clear();
    }
}
