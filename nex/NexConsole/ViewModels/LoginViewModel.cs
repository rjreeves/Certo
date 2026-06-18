using System;
using System.Threading.Tasks;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using NexConsole.Services;

namespace NexConsole.ViewModels;

public partial class LoginViewModel : ObservableObject
{
    private readonly ConfigService _config = new();

    // Initialised on a background thread so BCrypt seeding never blocks the UI
    private readonly Task<SecurityService> _securityTask = SecurityService.InitAsync;

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
            // Await the background init (instant if already done, waits if still hashing)
            var security = await _securityTask;
            var result   = await Task.Run(() => security.Authenticate(Username, Password));

            if (!result.Success)
            {
                ErrorMessage = result.Error;
                return;
            }

            var selectedRole = string.IsNullOrWhiteSpace(SelectedRole) ? "Application" : SelectedRole;
            if (RoleLevel.For(selectedRole) > RoleLevel.For(result.Role))
            {
                ErrorMessage = $"Your account is not authorised for the '{selectedRole}' role.";
                return;
            }

            AppSession.Current.Begin(Username, selectedRole);

            if (RememberMe)
                _config.SaveLogin(Username, selectedRole, Password);

            if (result.MustChangePassword)
                MustChangePassword?.Invoke(Username);
            else
                LoginSucceeded?.Invoke(selectedRole);
        }
        finally
        {
            IsBusy = false;
        }
    }
}
