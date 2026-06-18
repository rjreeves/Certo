using System;

namespace NexConsole.Models;

public class NotificationItem
{
    public string Id { get; set; } = "";
    public string Title { get; set; } = "";
    public string Body { get; set; } = "";
    public string Level { get; set; } = "";
    public DateTime CreatedAt { get; set; }
    public bool Read { get; set; }
}
