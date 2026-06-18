using Avalonia.Controls;
using NexConsole.ViewModels;

namespace NexConsole.Views;

public partial class UserEditWindow : Window
{
    public UserEditWindow() => InitializeComponent();

    public UserEditWindow(UserEditViewModel vm) : this()
    {
        DataContext = vm;
        vm.SaveSucceeded += () => Close(true);
        vm.Cancelled     += () => Close(false);
    }
}
