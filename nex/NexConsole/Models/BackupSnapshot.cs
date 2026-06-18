using System;

namespace NexConsole.Models;

public class BackupSnapshot
{
    public string Id { get; set; } = "";
    public string Label { get; set; } = "";
    public string State { get; set; } = "";
    public double? SizeGb { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime? VerifiedAt { get; set; }
}
