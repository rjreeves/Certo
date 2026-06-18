using CommunityToolkit.Mvvm.Input;
using NexConsole.Services;
using NexConsole.Views;

namespace NexConsole.ViewModels;

public partial class SettingsViewModel : ViewModelBase
{
    public AppSession Session => AppSession.Current;
    public UserManagementViewModel UserManagement { get; } = new();

    [RelayCommand]
    private void ChangePassword()
    {
        var vm  = new ChangePasswordViewModel(Session.Username, isForced: false);
        var win = new ChangePasswordWindow(vm);
        win.Show();
    }
}
