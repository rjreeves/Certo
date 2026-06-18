using System;
using System.IO;
using System.Runtime.InteropServices;
using Avalonia.Media;
using Avalonia.Threading;
using CommunityToolkit.Mvvm.ComponentModel;

namespace NexConsole.ViewModels;

public partial class SystemResourcesViewModel : ViewModelBase
{
    [ObservableProperty] private string _ramText = "Measuring...";
    [ObservableProperty] private double _ramUsed;
    [ObservableProperty] private IBrush _ramColor = new SolidColorBrush(Color.Parse("#555555"));

    [ObservableProperty] private string _diskText = "Measuring...";
    [ObservableProperty] private double _diskUsed;
    [ObservableProperty] private IBrush _diskColor = new SolidColorBrush(Color.Parse("#555555"));

    public SystemResourcesViewModel()
    {
        Refresh();
        var timer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(10) };
        timer.Tick += (_, _) => Refresh();
        timer.Start();
    }

    private void Refresh()
    {
        RefreshRam();
        RefreshDisk();
    }

    private void RefreshRam()
    {
        try
        {
            var mem = new MEMORYSTATUSEX();
            mem.dwLength = (uint)Marshal.SizeOf(mem);
            GlobalMemoryStatusEx(ref mem);

            var totalGb = mem.ullTotalPhys / (1024.0 * 1024 * 1024);
            var freeGb  = mem.ullAvailPhys / (1024.0 * 1024 * 1024);
            var freePct = freeGb / totalGb * 100;

            RamText  = $"{freeGb:F1} GB free  /  {totalGb:F0} GB total";
            RamUsed  = 100 - freePct;
            RamColor = BrushFromFree(freePct);
        }
        catch { RamText = "Unavailable"; }
    }

    private void RefreshDisk()
    {
        try
        {
            var root  = Path.GetPathRoot(Environment.GetFolderPath(Environment.SpecialFolder.System))!;
            var drive = new DriveInfo(root);
            var totalGb = drive.TotalSize / (1024.0 * 1024 * 1024);
            var freeGb  = drive.AvailableFreeSpace / (1024.0 * 1024 * 1024);
            var freePct = freeGb / totalGb * 100;

            DiskText  = $"{freeGb:F0} GB free  /  {totalGb:F0} GB total";
            DiskUsed  = 100 - freePct;
            DiskColor = BrushFromFree(freePct);
        }
        catch { DiskText = "Unavailable"; }
    }

    private static IBrush BrushFromFree(double freePct) => freePct switch
    {
        >= 50 => new SolidColorBrush(Color.Parse("#4caf7d")),
        >= 20 => new SolidColorBrush(Color.Parse("#f0a030")),
        _     => new SolidColorBrush(Color.Parse("#e25c5c")),
    };

    [StructLayout(LayoutKind.Sequential)]
    private struct MEMORYSTATUSEX
    {
        public uint  dwLength;
        public uint  dwMemoryLoad;
        public ulong ullTotalPhys;
        public ulong ullAvailPhys;
        public ulong ullTotalPageFile;
        public ulong ullAvailPageFile;
        public ulong ullTotalVirtual;
        public ulong ullAvailVirtual;
        public ulong ullAvailExtendedVirtual;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GlobalMemoryStatusEx(ref MEMORYSTATUSEX lpBuffer);
}
