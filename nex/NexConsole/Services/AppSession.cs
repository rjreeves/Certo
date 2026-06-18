using CommunityToolkit.Mvvm.ComponentModel;

namespace NexConsole.Services;

public partial class AppSession : ObservableObject
{
    public static AppSession Current { get; } = new();

    [ObservableProperty] private string _username = "";
    [ObservableProperty] private string _role = "";

    public int  Level          => RoleLevel.For(Role);
    public bool IsOwner        => Level >= RoleLevel.Owner;
    public bool IsAdministrator => Level >= RoleLevel.Administrator;
    public bool CanManage      => Level >= RoleLevel.Administrator; // L3+

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
