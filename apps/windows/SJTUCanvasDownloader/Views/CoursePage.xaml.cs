using System.ComponentModel;
using CanvasDownloader.Services;
using CanvasDownloader.ViewModels;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;

namespace CanvasDownloader.Views;

public sealed partial class CoursePage : Page
{
    private string _tab = "lessons";
    private bool _historicalDismissed;

    public CoursePage()
    {
        InitializeComponent();
        HistoricalBar.Closed += (_, _) => _historicalDismissed = true;
    }

    /// <summary>Opens or closes a message; a closed one takes no space.</summary>
    private static void Show(InfoBar bar, bool open)
    {
        bar.IsOpen = open;
        bar.Visibility = open ? Visibility.Visible : Visibility.Collapsed;
    }

    private void OnInfoBarClosed(InfoBar sender, InfoBarClosedEventArgs args) => sender.Visibility = Visibility.Collapsed;

    public CourseViewModel ViewModel { get; private set; } = new(new Course());

    protected override async void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        if (e.Parameter is Course course)
        {
            ViewModel = new CourseViewModel(course);
            Bindings.Update();
        }
        ViewModel.PropertyChanged += OnViewModelChanged;
        SyncTrackToggles();
        UpdateView();
        await ViewModel.LoadAsync();
        UpdateView();
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        base.OnNavigatedFrom(e);
        ViewModel.PropertyChanged -= OnViewModelChanged;
        ViewModel.Dispose();
    }

    private void OnViewModelChanged(object? sender, PropertyChangedEventArgs e) => UpdateView();

    private bool OnLessons => _tab == "lessons";

    private void UpdateView()
    {
        var lessons = OnLessons;
        LessonList.Visibility = lessons ? Visibility.Visible : Visibility.Collapsed;
        FileList.Visibility = lessons ? Visibility.Collapsed : Visibility.Visible;
        TracksButton.Visibility = lessons ? Visibility.Visible : Visibility.Collapsed;
        SearchBox.PlaceholderText = lessons ? "搜索录像" : "搜索文件";
        LessonsTab.Text = ViewModel.LessonsLoading || ViewModel.LessonsError is not null ? "课堂录像" : $"课堂录像 {ViewModel.LessonCount}";
        FilesTab.Text = ViewModel.FilesLoading || ViewModel.FilesError is not null ? "课程文件" : $"课程文件 {ViewModel.FileCount}";

        var loading = lessons ? ViewModel.LessonsLoading : ViewModel.FilesLoading;
        LoadingRing.IsActive = loading;
        LoadingRing.Visibility = loading ? Visibility.Visible : Visibility.Collapsed;

        Show(LessonsErrorBar, lessons && ViewModel.LessonsError is not null);
        LessonsErrorBar.Severity = ViewModel.LessonsNotice ? InfoBarSeverity.Informational : InfoBarSeverity.Error;
        LessonsErrorBar.Title = ViewModel.LessonsNotice ? "暂时没有课堂录像" : "无法读取课堂录像";
        LessonsErrorBar.Message = ViewModel.LessonsError is { } error
            ? error + (ViewModel.RetrySeconds > 0 ? $"（{ViewModel.RetrySeconds} 秒后自动重试）" : "")
            : "";
        LessonsRetryButton.IsEnabled = ViewModel.RetrySeconds == 0;
        Show(HistoricalBar, lessons && !_historicalDismissed && ViewModel.HistoricalCount > 0 && ViewModel.LessonsError is null);
        HistoricalBar.Message = $"新视频平台没有这门课的录像，下面的 {ViewModel.HistoricalCount} 条录像来自旧版课堂视频。";
        Show(FilesErrorBar, !lessons && ViewModel.FilesError is not null);
        FilesErrorBar.Message = ViewModel.FilesError ?? "";

        var empty = lessons ? ViewModel.LessonsEmpty : ViewModel.FilesEmpty;
        EmptyState.Visibility = empty ? Visibility.Visible : Visibility.Collapsed;
        var searching = ViewModel.Query.Trim().Length > 0;
        EmptyGlyph.Glyph = lessons ? "\uE714" : "\uE8A5";
        EmptyTitle.Text = searching ? "没有匹配的结果" : lessons ? "还没有课堂录像" : "还没有课程文件";
        EmptyDetail.Text = searching
            ? "换个关键词，或清空搜索查看完整列表。"
            : lessons
                ? "视频平台暂未返回这门课的录像。录像通常在课后几小时内开放。"
                : "教师发布资料后会出现在这里。";
        UpdateSelectionUi();
    }

    private void OnTabChanged(SelectorBar sender, SelectorBarSelectionChangedEventArgs args)
    {
        _tab = sender.SelectedItem?.Tag as string ?? "lessons";
        LessonList.SelectedItems.Clear();
        FileList.SelectedItems.Clear();
        UpdateView();
    }

    private void OnSearchChanged(AutoSuggestBox sender, AutoSuggestBoxTextChangedEventArgs args)
    {
        if (args.Reason == AutoSuggestionBoxTextChangeReason.UserInput)
        {
            ViewModel.Query = sender.Text;
        }
    }

    private void OnTrackToggled(object sender, RoutedEventArgs e)
    {
        if (sender is ToggleMenuFlyoutItem { Tag: string track } item)
        {
            ViewModel.SetTrack(track, item.IsChecked);
        }
        SyncTrackToggles();
        UpdateSelectionUi();
    }

    /// <summary>The last selected track cannot be switched off.</summary>
    private void SyncTrackToggles()
    {
        SlidesToggle.IsChecked = ViewModel.HasTrack("slides");
        TeacherToggle.IsChecked = ViewModel.HasTrack("teacher");
        CompositeToggle.IsChecked = ViewModel.HasTrack("composite");
    }

    private void OnLessonContainerChanging(ListViewBase sender, ContainerContentChangingEventArgs args)
    {
        if (args.Item is not LessonItemViewModel lesson)
        {
            return;
        }
        if (args.InRecycleQueue)
        {
            ViewModel.SetRealized(lesson, false);
            return;
        }
        // Lessons that are not open yet cannot be selected or downloaded.
        args.ItemContainer.IsEnabled = lesson.Available;
        ViewModel.SetRealized(lesson, true);
    }

    private void OnSelectionChanged(object sender, SelectionChangedEventArgs e) => UpdateSelectionUi();

    private List<LessonItemViewModel> SelectedLessons => LessonList.SelectedItems.OfType<LessonItemViewModel>().Where(lesson => lesson.Available).ToList();

    private List<FileItemViewModel> SelectedFiles => FileList.SelectedItems.OfType<FileItemViewModel>().ToList();

    private void UpdateSelectionUi()
    {
        int selected, selectable;
        string text;
        if (OnLessons)
        {
            selected = SelectedLessons.Count;
            selectable = ViewModel.Lessons.Count(lesson => lesson.Available);
            var tasks = selected * ViewModel.Tracks.Count;
            text = selected == 0 ? "" : $"已选择 {selected} 讲 · 将下载 {tasks} 个视频文件";
            DownloadButtonText.Text = selected == 0 ? "下载所选" : $"下载 {selected} 讲";
        }
        else
        {
            selected = SelectedFiles.Count;
            selectable = ViewModel.Files.Count;
            var bytes = SelectedFiles.Sum(file => file.File.Size);
            text = selected == 0 ? "" : $"已选择 {selected} 个文件 · {Helpers.Format.Size(bytes)}";
            DownloadButtonText.Text = selected == 0 ? "下载所选" : $"下载 {selected} 个文件";
        }
        SelectionText.Text = text;
        DownloadButton.IsEnabled = selected > 0 && AppHost.IsSignedIn;
        SelectAllBox.IsEnabled = selectable > 0;
        SelectAllBox.IsChecked = selected == 0 ? false : selected >= selectable ? true : null;
    }

    private void OnSelectAll(object sender, RoutedEventArgs e)
    {
        var list = OnLessons ? LessonList : FileList;
        if (SelectAllBox.IsChecked == true)
        {
            var items = OnLessons
                ? ViewModel.Lessons.Where(lesson => lesson.Available).Cast<object>()
                : ViewModel.Files.Cast<object>();
            foreach (var item in items)
            {
                if (!list.SelectedItems.Contains(item))
                {
                    list.SelectedItems.Add(item);
                }
            }
        }
        else
        {
            list.SelectedItems.Clear();
        }
        UpdateSelectionUi();
    }

    private async void OnDownloadSelected(object sender, RoutedEventArgs e)
    {
        var items = OnLessons ? ViewModel.DownloadsFor(SelectedLessons) : ViewModel.DownloadsFor(SelectedFiles);
        if (await QueueAsync(items))
        {
            (OnLessons ? LessonList : FileList).SelectedItems.Clear();
        }
    }

    private async void OnDownloadLesson(object sender, RoutedEventArgs e)
    {
        if (sender is FrameworkElement { Tag: LessonItemViewModel lesson })
        {
            await QueueAsync(ViewModel.DownloadsFor([lesson]));
        }
    }

    private async void OnDownloadFile(object sender, RoutedEventArgs e)
    {
        if (sender is FrameworkElement { Tag: FileItemViewModel file })
        {
            await QueueAsync(ViewModel.DownloadsFor([file]));
        }
    }

    private async Task<bool> QueueAsync(List<NewDownload> items)
    {
        if (items.Count == 0)
        {
            return false;
        }
        DownloadButton.IsEnabled = false;
        try
        {
            var result = await DownloadActions.CreateAsync(items);
            if (result is null)
            {
                return false;
            }
            ResultBar.Severity = result.Created.Count == 0 ? InfoBarSeverity.Warning
                : result.Skipped.Count > 0 ? InfoBarSeverity.Warning : InfoBarSeverity.Success;
            ResultBar.Title = result.Created.Count > 0 ? "已加入下载" : "没有新的下载任务";
            ResultBar.Message = DownloadActions.Summary(result);
            Show(ResultBar, true);
            return result.Created.Count > 0;
        }
        catch (EngineException error)
        {
            ResultBar.Severity = InfoBarSeverity.Error;
            ResultBar.Title = "无法添加下载";
            ResultBar.Message = error.Message;
            Show(ResultBar, true);
            return false;
        }
        finally
        {
            UpdateSelectionUi();
        }
    }

    private void OnRetrySize(object sender, RoutedEventArgs e)
    {
        if (sender is FrameworkElement { Tag: LessonItemViewModel lesson })
        {
            ViewModel.RetrySize(lesson);
        }
    }

    private async void OnRetryLessons(object sender, RoutedEventArgs e) => await ViewModel.LoadLessonsAsync();

    private async void OnRetryFiles(object sender, RoutedEventArgs e) => await ViewModel.LoadFilesAsync();

    private void OnViewDownloads(object sender, RoutedEventArgs e) => App.Window.Navigate(typeof(DownloadsPage));
}
