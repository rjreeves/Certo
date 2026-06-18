using System;
using System.Linq;
using System.Threading.Tasks;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using NexConsole.Services;

namespace NexConsole.ViewModels;

public partial class UserEditViewModel : ObservableObject
{
    private readonly Task<SecurityService> _securityTask = SecurityService.InitAsync;
    private readonly int _editId; // 0 = new user

    public bool IsNew => _editId == 0;
    public string Title => IsNew ? "Add User" : "Edit User";

    // Editors can only assign roles up to their own level
    public string[] Roles { get; } = BuildRoles();

    private static string[] BuildRoles()
    {
        var editorLevel = AppSession.Current.Level;
        return new[] { "Administrator", "Application", "Owner", "ReadOnly" }
            .Where(r => RoleLevel.For(r) <= editorLevel)
            .OrderBy(r => r)
            .ToArray();
    }

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(SaveCommand))]
    private string _username = "";

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(SaveCommand))]
    private string _password = "";

    [ObservableProperty]
    private string _selectedRole = "Application";

    [ObservableProperty]
    private bool _mustChangePassword = true;

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(SaveCommand))]
    private bool _isBusy;

    [ObservableProperty]
    private string _errorMessage = "";

    public event Action? SaveSucceeded;
    public event Action? Cancelled;

    // New user
    public UserEditViewModel() => _editId = 0;

    // Edit existing
    public UserEditViewModel(UserRecord user)
    {
        _editId             = user.Id;
        _username           = user.Username;
        _selectedRole       = user.Role;
        _mustChangePassword = user.MustChangePwd;
    }

    private bool CanSave =>
        !IsBusy &&
        Username.Trim().Length > 0 &&
        (IsNew ? Password.Length >= 6 : true);

    [RelayCommand(CanExecute = nameof(CanSave))]
    private async Task Save()
    {
        ErrorMessage = "";
        IsBusy = true;
        try
        {
            var security = await _securityTask;
            if (IsNew)
            {
                var (ok, err) = await Task.Run(() =>
                    security.AddUser(Username.Trim(), Password, SelectedRole));
                if (!ok) { ErrorMessage = err; return; }
            }
            else
            {
                await Task.Run(() =>
                    security.UpdateUser(_editId, SelectedRole, MustChangePassword));
            }
            SaveSucceeded?.Invoke();
        }
        finally { IsBusy = false; }
    }

    [RelayCommand]
    private void Cancel() => Cancelled?.Invoke();
}
