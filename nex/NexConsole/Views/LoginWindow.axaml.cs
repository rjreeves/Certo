using Avalonia.Controls;
using NexConsole.ViewModels;

namespace NexConsole.Views;

public partial class LoginWindow : Window
{
    public LoginWindow() => InitializeComponent();

    public LoginWindow(LoginViewModel vm) : this()
    {
        DataContext = vm;

        vm.LoginSucceeded += _ => OpenMain();

        vm.MustChangePassword += async username =>
        {
            var cpVm = new ChangePasswordViewModel(username, isForced: true);
            var cpWin = new ChangePasswordWindow(cpVm);

            cpVm.ChangeSucceeded += () => { cpWin.Close(); OpenMain(); };

            await cpWin.ShowDialog(this);
        };
    }

    private void OpenMain()
    {
        var vm   = new MainWindowViewModel();
        var main = new MainWindow(vm);
        main.Show();
        Close();
    }
}
