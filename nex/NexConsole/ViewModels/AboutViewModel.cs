using System.Reflection;

namespace NexConsole.ViewModels;

public class AboutViewModel : ViewModelBase
{
    public string Version        { get; }
    public string VersionDisplay { get; }

    public AboutViewModel()
    {
        var v = Assembly.GetEntryAssembly()?.GetName().Version;
        Version        = v is null ? "1.0.0" : $"{v.Major}.{v.Minor}.{v.Build}";
        VersionDisplay = $"v{Version}";
    }
}
