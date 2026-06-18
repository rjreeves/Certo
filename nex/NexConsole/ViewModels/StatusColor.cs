using Avalonia.Media;

namespace NexConsole.ViewModels;

public static class StatusColor
{
    public static IBrush For(string status) => status.ToLowerInvariant() switch
    {
        "running" or "healthy" or "success" or "complete" or "ready" or "online" or "found"
            => new SolidColorBrush(Color.Parse("#4caf7d")),
        "warning" or "degraded" or "pending" or "queued"
            => new SolidColorBrush(Color.Parse("#f0a030")),
        "offline" or "stopped" or "failed" or "error" or "not found"
            => new SolidColorBrush(Color.Parse("#e25c5c")),
        _ => new SolidColorBrush(Color.Parse("#555555")),
    };
}
