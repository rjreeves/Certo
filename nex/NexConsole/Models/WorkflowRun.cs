using System;

namespace NexConsole.Models;

public class WorkflowRun
{
    public string Id { get; set; } = "";
    public string WorkflowName { get; set; } = "";
    public string State { get; set; } = "";
    public DateTime StartedAt { get; set; }
    public int? DurationMs { get; set; }
}
