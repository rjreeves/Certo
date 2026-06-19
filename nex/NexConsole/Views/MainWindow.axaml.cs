using System;
using Avalonia.Controls;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using NexConsole.ViewModels;

namespace NexConsole.Views;

public partial class MainWindow : Window
{
    public MainWindow()
    {
        InitializeComponent();
        try
        {
            var iconUri = new Uri("avares://NexConsole/Assets/syntra-icon-32.png");
            using var iconStream = AssetLoader.Open(iconUri);
            Icon = new Avalonia.Controls.WindowIcon(iconStream);
        }
        catch { /* keep default icon */ }
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