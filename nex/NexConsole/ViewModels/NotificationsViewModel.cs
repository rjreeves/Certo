using System.Collections.ObjectModel;
using System.Linq;
using Avalonia.Media;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using NexConsole.Services;

namespace NexConsole.ViewModels;

public partial class NotificationsViewModel : ViewModelBase
{
    public ObservableCollection<NotificationViewModel> Items { get; }

    [ObservableProperty]
    private int _unreadCount;

    public NotificationsViewModel()
    {
        Items = new ObservableCollection<NotificationViewModel>(
            MockDataService.NotificationItems.Select(n => new NotificationViewModel(n)));
        UnreadCount = Items.Count(i => !i.IsRead);  // Linq Count with predicate
    }

    [RelayCommand]
    private void MarkAllRead()
    {
        foreach (var item in Items)
            item.IsRead = true;
        UnreadCount = 0;
    }
}

public partial class NotificationViewModel : ObservableObject
{
    public string Title { get; }
    public string Body { get; }
    public string Level { get; }
    public IBrush LevelBrush { get; }
    public string CreatedText { get; }

    [ObservableProperty]
    private bool _isRead;

    public NotificationViewModel(NexConsole.Models.NotificationItem n)
    {
        Title = n.Title;
        Body = n.Body;
        Level = n.Level;
        LevelBrush = StatusColor.For(n.Level);
        CreatedText = n.CreatedAt.ToString("MMM d, HH:mm");
        _isRead = n.Read;
    }
}
