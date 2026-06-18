using Avalonia.Controls;
using NexConsole.ViewModels;

namespace NexConsole.Views;

public partial class ChangePasswordWindow : Window
{
    public ChangePasswordWindow() => InitializeComponent();

    public ChangePasswordWindow(ChangePasswordViewModel vm) : this()
    {
        DataContext = vm;

        vm.ChangeSucceeded += () => Close(true);
        vm.Cancelled       += () => Close(false);
    }
}
