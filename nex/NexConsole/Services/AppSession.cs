using CommunityToolkit.Mvvm.ComponentModel;

namespace NexConsole.Services;

public partial class AppSession : ObservableObject
{
    public static AppSession Current { get; } = new();

    [ObservableProperty] private string _username = "";
    [ObservableProperty] private string _role = "";

    public bool IsAdmin    => Role == "Admin";
    public bool IsOperator => Role == "Operator";
    public bool IsViewer   => Role == "Viewer";

    public void Begin(string username, string role)
    {
        Username = username;
        Role     = role;
    }

    public void End()
    {
        Username = "";
        Role     = "";
    }
}
