using System.Collections.ObjectModel;
using Avalonia.Media;
using CommunityToolkit.Mvvm.Input;
using NexConsole.Services;

namespace NexConsole.ViewModels;

public partial class LogsViewModel : ViewModelBase
{
    public ObservableCollection<LogEntryViewModel> Entries { get; } = [];

    public LogsViewModel()
    {
        Reload();
    }

    [RelayCommand]
    private void Refresh()
    {
        Reload();
    }

    private void Reload()
    {
        Entries.Clear();
        foreach (var e in MockDataService.LogEntries)
            Entries.Add(new LogEntryViewModel(e));
    }
}

public class LogEntryViewModel
{
    public string TimestampText { get; }
    public string Level { get; }
    public IBrush LevelBrush { get; }
    public string Source { get; }
    public string Message { get; }

    public LogEntryViewModel(NexConsole.Models.LogEntry e)
    {
        TimestampText = e.Timestamp.ToString("MM-dd HH:mm:ss");
        Level = e.Level.ToUpperInvariant();
        LevelBrush = e.Level.ToLowerInvariant() switch
        {
            "error"  => new SolidColorBrush(Color.Parse("#e25c5c")),
            "warn"   => new SolidColorBrush(Color.Parse("#f0a030")),
            "info"   => new SolidColorBrush(Color.Parse("#7ab3e0")),
            "debug"  => new SolidColorBrush(Color.Parse("#888888")),
            "trace"  => new SolidColorBrush(Color.Parse("#555555")),
            _        => new SolidColorBrush(Color.Parse("#aaaaaa")),
        };
        Source = e.Source;
        Message = e.Message;
    }
}
