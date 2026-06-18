using System.Collections.ObjectModel;
using System.Linq;
using Avalonia.Media;
using NexConsole.Services;

namespace NexConsole.ViewModels;

public partial class DashboardViewModel : ViewModelBase
{
    public SystemResourcesViewModel  Resources { get; } = new();
    public ConnectionProbeViewModel? Probe     { get; }
    public ObservableCollection<ServiceStatusViewModel> Services { get; }

    public DashboardViewModel()
    {
        var first = new ConfigService().Config.Connections
            .Find(c => !string.IsNullOrWhiteSpace(c.ConnectionString));

        if (first != null)
            Probe = new ConnectionProbeViewModel(first.Name, first.ConnectionString);

        Services = new ObservableCollection<ServiceStatusViewModel>(
            MockDataService.ServiceStatuses.Select(s => new ServiceStatusViewModel(s)));
    }
}

public class ServiceStatusViewModel
{
    public string Name { get; }
    public string State { get; }
    public IBrush StateBrush { get; }
    public string Uptime { get; }

    public ServiceStatusViewModel(NexConsole.Models.ServiceStatus s)
    {
        Name = s.Name;
        State = s.State;
        StateBrush = StatusColor.For(s.State);
        Uptime = s.Uptime;
    }
}
