using System;

namespace NexConsole.Models;

public class LogEntry
{
    public string Id { get; set; } = "";
    public string Level { get; set; } = "";
    public string Source { get; set; } = "";
    public string Message { get; set; } = "";
    public DateTime Timestamp { get; set; }
}
