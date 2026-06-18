using System;
using System.Collections.Generic;
using NexConsole.Models;

namespace NexConsole.Services;

public static class MockDataService
{
    private static readonly DateTime Now = DateTime.Now;

    public static readonly List<AppModule> AppModules =
    [
        new() { Id = "nex-studio",      Name = "Nex Studio",      Description = "Design and build tool",       Status = "ready",   Version = "v1.2.0", LastOpenedAt = Now.AddHours(-2)  },
        new() { Id = "restore-manager", Name = "Restore Manager", Description = "Snapshot restore utility",   Status = "running", Version = "v0.9.1", LastOpenedAt = Now.AddMinutes(-30) },
        new() { Id = "plugin-console",  Name = "Plugin Console",  Description = "Extension manager",          Status = "warning", Version = "v1.0.0", LastOpenedAt = Now.AddDays(-1)   },
        new() { Id = "log-indexer",     Name = "Log Indexer",     Description = "Real-time log indexing",     Status = "offline", Version = "v0.5.2", LastOpenedAt = Now.AddDays(-3)   },
    ];

    public static readonly List<ServiceStatus> ServiceStatuses =
    [
        new() { Id = "local-api",       Name = "Local API",       State = "healthy",  Uptime = "14d 3h",  LastCheckedAt = Now.ToString("o") },
        new() { Id = "scheduler",       Name = "Scheduler",       State = "healthy",  Uptime = "14d 3h",  LastCheckedAt = Now.ToString("o") },
        new() { Id = "backup-watcher",  Name = "Backup Watcher",  State = "degraded", Uptime = "2h 14m",  LastCheckedAt = Now.ToString("o") },
        new() { Id = "log-indexer-svc", Name = "Log Indexer",     State = "stopped",  Uptime = "—",       LastCheckedAt = Now.ToString("o") },
    ];

    public static readonly List<BackupSnapshot> BackupSnapshots =
    [
        new() { Id = "snap-2026-06-18-06", Label = "snap-2026-06-18-06", State = "complete", SizeGb = 3.8, CreatedAt = Now.Date.AddHours(6),              VerifiedAt = Now.Date.AddHours(7)  },
        new() { Id = "snap-2026-06-17-06", Label = "snap-2026-06-17-06", State = "complete", SizeGb = 3.6, CreatedAt = Now.Date.AddDays(-1).AddHours(6),  VerifiedAt = Now.Date.AddDays(-1).AddHours(7) },
        new() { Id = "snap-2026-06-16-06", Label = "snap-2026-06-16-06", State = "complete", SizeGb = 3.5, CreatedAt = Now.Date.AddDays(-2).AddHours(6),  VerifiedAt = Now.Date.AddDays(-2).AddHours(7) },
        new() { Id = "snap-manual-001",    Label = "snap-manual-001",    State = "failed",   SizeGb = null, CreatedAt = Now.Date.AddDays(-1).AddHours(14), VerifiedAt = null },
    ];

    public static readonly List<WorkflowRun> WorkflowRuns =
    [
        new() { Id = "wf-1", WorkflowName = "Nightly Backup", State = "success", StartedAt = Now.Date.AddDays(-1).AddHours(2),  DurationMs = 3400  },
        new() { Id = "wf-2", WorkflowName = "Health Check",   State = "success", StartedAt = Now.AddHours(-1),                  DurationMs = 812   },
        new() { Id = "wf-3", WorkflowName = "Index Rebuild",  State = "failed",  StartedAt = Now.AddHours(-2),                  DurationMs = 5200  },
        new() { Id = "wf-4", WorkflowName = "Report Export",  State = "running", StartedAt = Now.AddMinutes(-3),                DurationMs = null  },
        new() { Id = "wf-5", WorkflowName = "DB Vacuum",      State = "queued",  StartedAt = Now.AddMinutes(10),                DurationMs = null  },
    ];

    public static readonly List<NotificationItem> NotificationItems =
    [
        new() { Id = "n-1", Title = "Backup completed",             Body = "snap-2026-06-18-06 completed successfully.",          Level = "success", CreatedAt = Now.AddHours(-6),    Read = true  },
        new() { Id = "n-2", Title = "Disk space warning",           Body = "18% free on C: — consider cleaning up old snapshots.", Level = "warning", CreatedAt = Now.AddHours(-2),    Read = false },
        new() { Id = "n-3", Title = "Backup Watcher degraded",      Body = "Service is responding slowly. Check logs.",            Level = "error",   CreatedAt = Now.AddHours(-1),    Read = false },
        new() { Id = "n-4", Title = "Index rebuild failed",         Body = "Log Indexer encountered an unexpected error.",         Level = "error",   CreatedAt = Now.AddMinutes(-45), Read = false },
        new() { Id = "n-5", Title = "Health check passed",          Body = "All monitored endpoints responded within SLA.",        Level = "info",    CreatedAt = Now.AddMinutes(-30), Read = true  },
    ];

    public static readonly List<LogEntry> LogEntries =
    [
        new() { Id = "l-1", Level = "info",  Source = "health-monitor",  Message = "All services passed health check.",                   Timestamp = Now.AddMinutes(-30) },
        new() { Id = "l-2", Level = "warn",  Source = "backup-watcher",  Message = "Response latency exceeded 2000ms threshold.",          Timestamp = Now.AddHours(-1)   },
        new() { Id = "l-3", Level = "error", Source = "log-indexer",     Message = "Failed to open index file: permission denied.",         Timestamp = Now.AddHours(-2)   },
        new() { Id = "l-4", Level = "info",  Source = "scheduler",       Message = "Job 'Report Export' enqueued for immediate execution.", Timestamp = Now.AddMinutes(-3) },
        new() { Id = "l-5", Level = "debug", Source = "api",             Message = "GET /api/health responded 200 OK in 12ms.",             Timestamp = Now.AddMinutes(-5) },
        new() { Id = "l-6", Level = "trace", Source = "backup-watcher",  Message = "Snapshot directory scan started.",                      Timestamp = Now.AddHours(-6)   },
        new() { Id = "l-7", Level = "info",  Source = "scheduler",       Message = "Nightly Backup job completed in 3.4s.",                 Timestamp = Now.AddDays(-1).AddHours(2).AddSeconds(3) },
        new() { Id = "l-8", Level = "error", Source = "log-indexer",     Message = "Index rebuild aborted after 5200ms — see full trace.",  Timestamp = Now.AddHours(-2).AddSeconds(5) },
    ];
}
