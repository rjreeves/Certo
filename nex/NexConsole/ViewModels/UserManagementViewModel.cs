using System.Collections.ObjectModel;
using System.Linq;
using Avalonia.Media;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using NexConsole.Services;
using NexConsole.Views;

namespace NexConsole.ViewModels;

public partial class UserManagementViewModel : ObservableObject
{
    private readonly SecurityService _security = SecurityService.InitAsync.GetAwaiter().GetResult();

    public ObservableCollection<UserRowViewModel> Users { get; } = [];

    public UserManagementViewModel() => Reload();

    private void Reload()
    {
        Users.Clear();
        foreach (var u in _security.GetUsers())
            Users.Add(new UserRowViewModel(u, this));
    }

    [RelayCommand]
    private void AddUser()
    {
        var vm  = new UserEditViewModel();
        var win = new UserEditWindow(vm);
        vm.SaveSucceeded += () => { win.Close(); Reload(); };
        win.Show();
    }

    internal void OpenEdit(UserRowViewModel row)
    {
        var vm  = new UserEditViewModel(row.Record);
        var win = new UserEditWindow(vm);
        vm.SaveSucceeded += () => { win.Close(); Reload(); };
        win.Show();
    }

    internal void ToggleEnabled(UserRowViewModel row)
    {
        _security.SetEnabled(row.Record.Id, !row.Record.IsEnabled);
        Reload();
    }

    internal void Delete(UserRowViewModel row)
    {
        _security.DeleteUser(row.Record.Id);
        Reload();
    }
}

public partial class UserRowViewModel : ObservableObject
{
    private readonly UserManagementViewModel _parent;
    public UserRecord Record { get; }

    public string Username    => Record.Username;
    public string Role        => Record.Role;
    public string StatusText  => Record.IsEnabled ? "Active" : "Disabled";
    public IBrush StatusBrush => Record.IsEnabled
        ? new SolidColorBrush(Color.Parse("#4caf7d"))
        : new SolidColorBrush(Color.Parse("#555555"));
    public string ToggleLabel => Record.IsEnabled ? "Disable" : "Enable";
    public string CreatedAt   => Record.CreatedAt[..10]; // date portion only
    public bool   MustChangePwd => Record.MustChangePwd;

    public UserRowViewModel(UserRecord record, UserManagementViewModel parent)
    {
        Record  = record;
        _parent = parent;
    }

    public bool IsDisabled => !Record.IsEnabled;

    [RelayCommand] private void Edit()          => _parent.OpenEdit(this);
    [RelayCommand] private void ToggleEnabled() => _parent.ToggleEnabled(this);
    [RelayCommand] private void Delete()        => _parent.Delete(this);
}
