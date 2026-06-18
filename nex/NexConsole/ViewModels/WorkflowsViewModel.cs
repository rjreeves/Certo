using System.Collections.ObjectModel;
using System.Linq;
using Avalonia.Media;
using NexConsole.Services;

namespace NexConsole.ViewModels;

public partial class WorkflowsViewModel : ViewModelBase
{
    public ObservableCollection<WorkflowRunViewModel> Runs { get; }

    public WorkflowsViewModel()
    {
        Runs = new ObservableCollection<WorkflowRunViewModel>(
            MockDataService.WorkflowRuns.Select(r => new WorkflowRunViewModel(r)));
    }
}

public class WorkflowRunViewModel
{
    public string WorkflowName { get; }
    public string State { get; }
    public IBrush StateBrush { get; }
    public string DurationText { get; }
    public string StartedText { get; }

    public WorkflowRunViewModel(NexConsole.Models.WorkflowRun r)
    {
        WorkflowName = r.WorkflowName;
        State = r.State;
        StateBrush = StatusColor.For(r.State);
        DurationText = r.DurationMs.HasValue ? $"{r.DurationMs.Value / 1000.0:0.#}s" : "—";
        StartedText = r.StartedAt.ToString("MMM d, HH:mm");
    }
}
