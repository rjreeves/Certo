using System;
using System.Threading.Tasks;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using NexConsole.Services;

namespace NexConsole.ViewModels;

public partial class ChangePasswordViewModel : ObservableObject
{
    private readonly SecurityService _security = new();
    private readonly string _username;

    public string Username => _username;

    // When true this is a forced first-login reset — no current-password field shown
    public bool IsForced { get; }

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(ChangeCommand))]
    private string _currentPassword = "";

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(ChangeCommand))]
    private string _newPassword = "";

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(ChangeCommand))]
    private string _confirmPassword = "";

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(ChangeCommand))]
    private bool _isBusy;

    [ObservableProperty]
    private string _errorMessage = "";

    public event Action? ChangeSucceeded;
    public event Action? Cancelled;

    public ChangePasswordViewModel(string username, bool isForced = false)
    {
        _username = username;
        IsForced  = isForced;
    }

    private bool CanChange =>
        !IsBusy &&
        NewPassword.Length >= 6 &&
        NewPassword == ConfirmPassword &&
        (IsForced || CurrentPassword.Length > 0);

    [RelayCommand(CanExecute = nameof(CanChange))]
    private async Task Change()
    {
        ErrorMessage = "";

        if (NewPassword != ConfirmPassword)
        {
            ErrorMessage = "Passwords do not match.";
            return;
        }

        if (NewPassword.Length < 6)
        {
            ErrorMessage = "Password must be at least 6 characters.";
            return;
        }

        IsBusy = true;
        try
        {
            if (!IsForced)
            {
                // Verify current password before allowing change
                var check = await Task.Run(() => _security.Authenticate(_username, CurrentPassword));
                if (!check.Success)
                {
                    ErrorMessage = "Current password is incorrect.";
                    return;
                }
            }

            var ok = await Task.Run(() => _security.ChangePassword(_username, NewPassword));
            if (!ok)
            {
                ErrorMessage = "Failed to update password. Please try again.";
                return;
            }

            ChangeSucceeded?.Invoke();
        }
        finally
        {
            IsBusy = false;
        }
    }

    [RelayCommand]
    private void Cancel() => Cancelled?.Invoke();
}
