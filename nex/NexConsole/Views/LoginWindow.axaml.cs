using Avalonia.Controls;
using NexConsole.ViewModels;

namespace NexConsole.Views;

public partial class LoginWindow : Window
{
    public LoginWindow() => InitializeComponent();

    public LoginWindow(LoginViewModel vm) : this()
    {
        DataContext = vm;

        vm.LoginSucceeded += role => OpenMain();

        vm.MustChangePassword += username =>
        {
            var cpVm = new ChangePasswordViewModel(username, isForced: true);
            var cpWin = new ChangePasswordWindow(cpVm);

            cpVm.ChangeSucceeded += () => OpenMain();
            // Forced reset — no cancel path; window closes via ChangeSucceeded only

            cpWin.ShowDialog(this);
        };
    }

    private void OpenMain()
    {
        var main = new MainWindow { DataContext = new MainWindowViewModel() };
        main.Show();
        Close();
    }
}
