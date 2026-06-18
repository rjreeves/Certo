using System;
using System.Threading.Tasks;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using NexConsole.Services;

namespace NexConsole.ViewModels;

public partial class LoginViewModel : ObservableObject
{
    private readonly ConfigService _config = new();
    private readonly SecurityService _security = new();

    public string[] Roles { get; } = ["Administrator", "Application", "Owner", "ReadOnly"];

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(LoginCommand))]
    private string _username = "";

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(LoginCommand))]
    private string _password = "";

    [ObservableProperty]
    private string _selectedRole = "Application";

    [ObservableProperty]
    private bool _rememberMe;

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(LoginCommand))]
    private bool _isBusy;

    [ObservableProperty]
    private string _errorMessage = "";

    public event Action<string>? LoginSucceeded;
    public event Action<string>? MustChangePassword;

    public LoginViewModel()
    {
        var saved = _config.Config.Login;
        if (saved != null)
        {
            Username     = saved.Username;
            SelectedRole = Array.Exists(Roles, r => r == saved.Role) ? saved.Role : "Application";
            Password     = ConfigService.UnprotectPassword(saved.ProtectedPassword) ?? "";
            RememberMe   = true;
        }
    }

    // Unchecking Remember Me immediately removes the saved login
    partial void OnRememberMeChanged(bool value)
    {
        if (!value) _config.ClearLogin();
    }

    private bool CanLogin => Username.Length > 0 && Password.Length > 0 && !IsBusy;

    [RelayCommand(CanExecute = nameof(CanLogin))]
    private async Task Login()
    {
        ErrorMessage = "";
        IsBusy = true;

        try
        {
            var result = await Task.Run(() => _security.Authenticate(Username, Password));

            if (!result.Success)
            {
                ErrorMessage = result.Error;
                return;
            }

            // Role from the UI is the operator's chosen context; DB role is authoritative for auth
            var effectiveRole = string.IsNullOrWhiteSpace(SelectedRole) ? "Application" : SelectedRole;
            AppSession.Current.Begin(Username, effectiveRole);

            if (RememberMe)
                _config.SaveLogin(Username, effectiveRole, Password);

            if (result.MustChangePassword)
                MustChangePassword?.Invoke(Username);
            else
                LoginSucceeded?.Invoke(effectiveRole);
        }
        finally
        {
            IsBusy = false;
        }
    }
}
