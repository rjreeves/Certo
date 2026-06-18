namespace NexConsole.Services;

public static class RoleLevel
{
    public const int ReadOnly      = 1;
    public const int Application   = 2;
    public const int Administrator = 3;
    public const int Owner         = 4;

    public static int For(string role) => role switch
    {
        "Owner"         => Owner,
        "Administrator" => Administrator,
        "Application"   => Application,
        "ReadOnly"      => ReadOnly,
        _               => Application, // unknown → safe default
    };
}
