using System.Collections.ObjectModel;
using System.Linq;
using Avalonia.Media;
using NexConsole.Services;

namespace NexConsole.ViewModels;

public partial class ModulesViewModel : ViewModelBase
{
    public ObservableCollection<AppModuleViewModel> Modules { get; }

    public ModulesViewModel()
    {
        Modules = new ObservableCollection<AppModuleViewModel>(
            MockDataService.AppModules.Select(m => new AppModuleViewModel(m)));
    }
}

public class AppModuleViewModel
{
    public string Name { get; }
    public string Description { get; }
    public string Status { get; }
    public IBrush StatusBrush { get; }
    public string Version { get; }

    public AppModuleViewModel(NexConsole.Models.AppModule m)
    {
        Name = m.Name;
        Description = m.Description;
        Status = m.Status;
        StatusBrush = StatusColor.For(m.Status);
        Version = m.Version;
    }
}
