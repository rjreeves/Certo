using System.Collections.ObjectModel;
using System.Linq;
using Avalonia.Media;
using NexConsole.Services;

namespace NexConsole.ViewModels;

public partial class BackupsViewModel : ViewModelBase
{
    public ObservableCollection<BackupSnapshotViewModel> Snapshots { get; }

    public BackupsViewModel()
    {
        Snapshots = new ObservableCollection<BackupSnapshotViewModel>(
            MockDataService.BackupSnapshots.Select(s => new BackupSnapshotViewModel(s)));
    }
}

public class BackupSnapshotViewModel
{
    public string Label { get; }
    public string State { get; }
    public IBrush StateBrush { get; }
    public string SizeText { get; }
    public string CreatedText { get; }
    public string VerifiedText { get; }
    public IBrush VerifiedBrush { get; }

    public BackupSnapshotViewModel(NexConsole.Models.BackupSnapshot s)
    {
        Label = s.Label;
        State = s.State;
        StateBrush = StatusColor.For(s.State);
        SizeText = s.SizeGb.HasValue ? $"{s.SizeGb:0.#} GB" : "—";
        CreatedText = s.CreatedAt.ToString("MMM d, HH:mm");
        VerifiedText = s.VerifiedAt.HasValue ? "Verified" : "Not verified";
        VerifiedBrush = s.VerifiedAt.HasValue
            ? new SolidColorBrush(Color.Parse("#4caf7d"))
            : new SolidColorBrush(Color.Parse("#e25c5c"));
    }
}
