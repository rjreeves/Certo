using System;
using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using NexConsole.Services;

namespace NexConsole.ViewModels;

public partial class LoginViewModel : ObservableObject
{
    private readonly ConfigService _config = new();
    private readonly SecurityService _security = new();

    public ObservableCollection<string> Roles { get; }

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(LoginCommand))]
    private string _username = "";

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(LoginCommand))]
    private string _password = "";

    [ObservableProperty]
    private string _selectedRole = "Operator";

    [ObservableProperty]
    private bool _rememberMe;

    [ObservableProperty]
    private string _errorMessage = "";

    // Fired on successful login; carries the authenticated role
    public event Action<string>? LoginSucceeded;

    // Fired when the user must change their password before proceeding
    public event Action<string>? MustChangePassword;

    public LoginViewModel()
    {
        Roles = new ObservableCollection<string>(_security.GetRoles());
        if (Roles.Count == 0) Roles.Add("Admin"); // fallback before first DB write

        var saved = _config.Config.Login;
        if (saved != null)
        {
            Username = saved.Username;
            SelectedRole = Roles.Contains(saved.Role) ? saved.Role : Roles[0];
            RememberMe = true;
        }
        else
        {
            SelectedRole = Roles[0];
        }
    }

    private bool CanLogin => Username.Length > 0 && Password.Length > 0;

    [RelayCommand(CanExecute = nameof(CanLogin))]
    private void Login()
    {
        ErrorMessage = "";

        var result = _security.Authenticate(Username, Password);

        if (!result.Success)
        {
            ErrorMessage = result.Error;
            return;
        }

        AppSession.Current.Begin(Username, result.Role);

        if (RememberMe)
            _config.SaveLogin(Username, result.Role);
        else
            _config.ClearLogin();

        if (result.MustChangePassword)
            MustChangePassword?.Invoke(Username);
        else
            LoginSucceeded?.Invoke(result.Role);
    }
}
