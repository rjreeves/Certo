using System;

namespace NexConsole.Models;

public class AppModule
{
    public string Id { get; set; } = "";
    public string Name { get; set; } = "";
    public string Description { get; set; } = "";
    public string Status { get; set; } = "";
    public string Version { get; set; } = "";
    public DateTime LastOpenedAt { get; set; }
}
