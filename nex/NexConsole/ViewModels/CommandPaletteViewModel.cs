using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Linq;
using CommunityToolkit.Mvvm.ComponentModel;

namespace NexConsole.ViewModels;

public class PaletteCommand
{
    public string Title { get; set; } = "";
    public string Scope { get; set; } = "";
    public Action Execute { get; set; } = () => { };
}

public partial class CommandPaletteViewModel : ObservableObject
{
    private readonly List<PaletteCommand> _all;

    public ObservableCollection<PaletteCommand> Filtered { get; } = [];

    [ObservableProperty]
    private string _query = "";

    [ObservableProperty]
    private int _selectedIndex;

    public CommandPaletteViewModel(Action<string> navigateTo)
    {
        _all =
        [
            new() { Title = "Dashboard",      Scope = "Navigate", Execute = () => navigateTo("Dashboard")      },
            new() { Title = "Modules",         Scope = "Navigate", Execute = () => navigateTo("Modules")        },
            new() { Title = "Backups",         Scope = "Navigate", Execute = () => navigateTo("Backups")        },
            new() { Title = "Workflows",       Scope = "Navigate", Execute = () => navigateTo("Workflows")      },
            new() { Title = "Notifications",   Scope = "Navigate", Execute = () => navigateTo("Notifications")  },
            new() { Title = "Logs",            Scope = "Navigate", Execute = () => navigateTo("Logs")           },
            new() { Title = "Settings",        Scope = "Navigate", Execute = () => navigateTo("Settings")       },
        ];
        RebuildFiltered();
    }

    partial void OnQueryChanged(string value) => RebuildFiltered();

    private void RebuildFiltered()
    {
        Filtered.Clear();
        var q = Query.Trim().ToLowerInvariant();
        var matches = string.IsNullOrEmpty(q)
            ? _all
            : _all.Where(c => c.Title.ToLowerInvariant().Contains(q));
        foreach (var c in matches)
            Filtered.Add(c);
        SelectedIndex = Filtered.Count > 0 ? 0 : -1;
    }

    public void SelectNext()
    {
        if (Filtered.Count == 0) return;
        SelectedIndex = (SelectedIndex + 1) % Filtered.Count;
    }

    public void SelectPrev()
    {
        if (Filtered.Count == 0) return;
        SelectedIndex = (SelectedIndex - 1 + Filtered.Count) % Filtered.Count;
    }

    public void ExecuteSelected()
    {
        if (SelectedIndex >= 0 && SelectedIndex < Filtered.Count)
            Filtered[SelectedIndex].Execute();
    }
}
