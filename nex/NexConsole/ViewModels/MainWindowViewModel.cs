using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using NexConsole.Services;

namespace NexConsole.ViewModels;

public partial class MainWindowViewModel : ViewModelBase
{
    public AppSession Session => AppSession.Current;

    [RelayCommand]
    private void ChangePassword()
    {
        var vm  = new ChangePasswordViewModel(Session.Username, isForced: false);
        var win = new Views.ChangePasswordWindow(vm);
        win.Show();
    }

    [ObservableProperty]
    private ViewModelBase _currentPage;

    public ObservableCollection<NavItem> NavItems { get; } =
    [
        new("Dashboard",     "M3 13h8V3H3v10zm0 8h8v-6H3v6zm10 0h8V11h-8v10zm0-18v6h8V3h-8z",                                                                                                                                                                                                                                                                                                                                                                                                          RoleLevel.ReadOnly),
        new("Workflows",     "M17 12h-5v5h5v-5zM16 1v2H8V1H6v2H5c-1.11 0-1.99.9-1.99 2L3 19c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2h-1V1h-2zm3 18H5V8h14v11z",                                                                                                                                                                                                                                                                                                                             RoleLevel.ReadOnly),
        new("Notifications", "M12 22c1.1 0 2-.9 2-2h-4c0 1.1.9 2 2 2zm6-6v-5c0-3.07-1.63-5.64-4.5-6.32V4c0-.83-.67-1.5-1.5-1.5s-1.5.67-1.5 1.5v.68C7.64 5.36 6 7.92 6 11v5l-2 2v1h16v-1l-2-2z",                                                                                                                                                                                                                                                                                                     RoleLevel.ReadOnly),
        new("Backups",       "M20 6h-2.18c.07-.44.18-.88.18-1.35C18 2.99 16.4 1.5 14.5 1.5c-1.12 0-2.06.61-2.66 1.5H12c-.6-.9-1.54-1.5-2.67-1.5C7.6 1.5 6 3 6 4.65c0 .47.1.91.18 1.35H4c-1.1 0-2 .9-2 2v11c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2z",                                                                                                                                                                                                                                      RoleLevel.ReadOnly),
        new("Modules",       "M4 8h4V4H4v4zm6 12h4v-4h-4v4zm-6 0h4v-4H4v4zm0-6h4v-4H4v4zm6 0h4v-4h-4v4zm6-10v4h4V4h-4zm-6 4h4V4h-4v4zm6 6h4v-4h-4v4zm0 6h4v-4h-4v4z",                                                                                                                                                                                                                                                                                                                               RoleLevel.ReadOnly),
        new("Logs",          "M14 2H6c-1.1 0-1.99.9-1.99 2L4 20c0 1.1.89 2 1.99 2H18c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z",                                                                                                                                                                                                                                                                                                                                       RoleLevel.ReadOnly),
        new("Settings",      "M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6z", RoleLevel.Administrator),
    ];

    public MainWindowViewModel()
    {
        _currentPage = new DashboardViewModel();
        NavItems[0].IsSelected = true;

        // Hide nav items the current user doesn't have level for
        foreach (var item in NavItems)
            item.IsVisible = Session.Level >= item.MinLevel;
    }

    [RelayCommand]
    private void Navigate(NavItem item)
    {
        // Hard guard — reject if session level is insufficient
        if (Session.Level < item.MinLevel) return;

        foreach (var nav in NavItems) nav.IsSelected = false;
        item.IsSelected = true;

        CurrentPage = item.Label switch
        {
            "Dashboard"     => new DashboardViewModel(),
            "Workflows"     => new WorkflowsViewModel(),
            "Notifications" => new NotificationsViewModel(),
            "Backups"       => new BackupsViewModel(),
            "Modules"       => new ModulesViewModel(),
            "Logs"          => new LogsViewModel(),
            "Settings"      => new SettingsViewModel(),
            _               => new DashboardViewModel(),
        };
    }
}

public partial class NavItem(string label, string iconPath, int minLevel) : ObservableObject
{
    public string Label    { get; } = label;
    public string IconPath { get; } = iconPath;
    public int    MinLevel { get; } = minLevel;

    [ObservableProperty] private bool _isSelected;
    [ObservableProperty] private bool _isVisible = true;
}
