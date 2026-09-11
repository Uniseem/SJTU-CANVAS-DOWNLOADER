using CanvasDownloader.Services;
using CanvasDownloader.ViewModels;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;

namespace CanvasDownloader.Views;

public sealed partial class CoursesPage : Page
{
    private bool _loaded;

    public CoursesPage()
    {
        InitializeComponent();
    }

    public CoursesViewModel ViewModel { get; } = new();

    protected override async void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        AppHost.AccountChanged += OnAccountChanged;
        // The page is cached: going back from a course keeps the list as it was.
        if (!_loaded)
        {
            _loaded = true;
            await ViewModel.LoadAsync();
            UpdateFilterLabels();
        }
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        base.OnNavigatedFrom(e);
        AppHost.AccountChanged -= OnAccountChanged;
    }

    private void OnAccountChanged()
    {
        // A different login lists different courses.
        _loaded = false;
    }

    private async void OnRefresh(object sender, RoutedEventArgs e)
    {
        await ViewModel.LoadAsync();
        UpdateFilterLabels();
    }

    private void OnFilterChanged(SelectorBar sender, SelectorBarSelectionChangedEventArgs args)
    {
        ViewModel.Filter = sender.SelectedItem?.Tag as string ?? "active";
    }

    private void OnSearchChanged(AutoSuggestBox sender, AutoSuggestBoxTextChangedEventArgs args)
    {
        if (args.Reason == AutoSuggestionBoxTextChangeReason.UserInput)
        {
            ViewModel.Query = sender.Text;
        }
    }

    private void UpdateFilterLabels()
    {
        ActiveFilter.Text = ViewModel.ActiveCount > 0 ? $"正在修读 {ViewModel.ActiveCount}" : "正在修读";
        CompletedFilter.Text = ViewModel.CompletedCount > 0 ? $"历史课程 {ViewModel.CompletedCount}" : "历史课程";
        var all = ViewModel.ActiveCount + ViewModel.CompletedCount;
        AllFilter.Text = all > 0 ? $"全部 {all}" : "全部";
    }

    private void OnCourseClick(object sender, ItemClickEventArgs e)
    {
        if (e.ClickedItem is CourseItemViewModel item)
        {
            App.Window.Navigate(typeof(CoursePage), item.Course);
        }
    }
}
