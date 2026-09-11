using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using CanvasDownloader.Services;

namespace CanvasDownloader.ViewModels;

/// <summary>One course card.</summary>
public sealed class CourseItemViewModel(Course course)
{
    public Course Course { get; } = course;

    public string Name => Course.Name;

    public string Subtitle => string.Join(" · ", new[] { Course.CourseCode, Course.Term }.Where(value => !string.IsNullOrWhiteSpace(value)));

    public string Teacher => string.IsNullOrWhiteSpace(Course.Teacher) ? "教师信息未提供" : Course.Teacher!;

    public bool IsPending => Course.EnrollmentState == "invited_or_pending";

    /// <summary>The name screen readers and UI Automation read for the card.</summary>
    public override string ToString() => Name;
}

public sealed partial class CoursesViewModel : ObservableObject
{
    private List<Course> _all = [];

    public ObservableCollection<CourseItemViewModel> Items { get; } = [];

    /// <summary>active | completed | all</summary>
    [ObservableProperty]
    public partial string Filter { get; set; } = "active";

    [ObservableProperty]
    public partial string Query { get; set; } = "";

    [ObservableProperty]
    public partial bool IsLoading { get; set; }

    [ObservableProperty]
    public partial bool IsEmpty { get; set; }

    [ObservableProperty]
    public partial string? Error { get; set; }

    [ObservableProperty]
    public partial int ActiveCount { get; set; }

    [ObservableProperty]
    public partial int CompletedCount { get; set; }

    public bool HasCourses => _all.Count > 0;

    public async Task LoadAsync()
    {
        if (!AppHost.IsSignedIn)
        {
            return;
        }
        IsLoading = true;
        Error = null;
        try
        {
            _all = (await AppHost.RequireEngine().CallAsync<CourseList>("courses.list")).Courses;
            ActiveCount = _all.Count(IsActive);
            CompletedCount = _all.Count - ActiveCount;
            Apply();
        }
        catch (EngineException error) when (!error.IsUnauthorized)
        {
            Error = error.Message;
        }
        catch (EngineException)
        {
            // The window switches to the login page.
        }
        finally
        {
            IsLoading = false;
            IsEmpty = Items.Count == 0 && Error is null;
        }
    }

    partial void OnFilterChanged(string value) => Apply();

    partial void OnQueryChanged(string value) => Apply();

    private static bool IsActive(Course course) => course.EnrollmentState != "completed";

    private void Apply()
    {
        var query = Query.Trim();
        var visible = _all.Where(course => Filter switch
            {
                "active" => IsActive(course),
                "completed" => !IsActive(course),
                _ => true,
            })
            .Where(course => query.Length == 0
                || course.Name.Contains(query, StringComparison.CurrentCultureIgnoreCase)
                || course.CourseCode.Contains(query, StringComparison.CurrentCultureIgnoreCase)
                || (course.Teacher ?? "").Contains(query, StringComparison.CurrentCultureIgnoreCase))
            .ToList();
        Items.Clear();
        foreach (var course in visible)
        {
            Items.Add(new CourseItemViewModel(course));
        }
        IsEmpty = Items.Count == 0 && !IsLoading && Error is null;
    }
}
