using System;
using System.Reflection;
using Avalonia.Controls;
using Avalonia.Media.Imaging;
using Avalonia.Platform;

namespace NexConsole.Views;

public partial class AboutView : UserControl
{
    public AboutView()
    {
        InitializeComponent();

        var v = Assembly.GetEntryAssembly()?.GetName().Version;
        VersionLabel.Text = $"v{(v is null ? "1.0.0" : $"{v.Major}.{v.Minor}.{v.Build}")}";

        try
        {
            var uri = new Uri("avares://NexConsole/Assets/Syntra-200.png");
            using var stream = AssetLoader.Open(uri);
            SyntraLogo.Source = new Bitmap(stream);
        }
        catch { /* logo is decorative */ }
    }
}
