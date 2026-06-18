using Avalonia.Controls;
using NexConsole.ViewModels;

namespace NexConsole.Views;

public partial class MainWindow : Window
{
    public MainWindow()
    {
        InitializeComponent();
    }

    public MainWindow(MainWindowViewModel vm) : this()
    {
        DataContext = vm;
        vm.LogoutRequested += () =>
        {
            var loginVm  = new LoginViewModel();
            var loginWin = new LoginWindow(loginVm);
            loginWin.Show();
            Close();
        };
    }
}