using Avalonia.Controls;
using NexConsole.ViewModels;

namespace NexConsole.Views;

public partial class LoginWindow : Window
{
    public LoginWindow()
    {
        InitializeComponent();
    }

    public LoginWindow(LoginViewModel vm) : this()
    {
        DataContext = vm;

        vm.LoginSucceeded += role =>
        {
            var main = new MainWindow { DataContext = new MainWindowViewModel() };
            main.Show();
            Close();
        };

        vm.MustChangePassword += username =>
        {
            // TODO: open a ChangePasswordDialog — for now surface the prompt via ErrorMessage
            vm.ErrorMessage = "First login detected — please change your password. (default: admin / admin)";
        };
    }
}
