using System.Text.Json;

namespace NotIde;

/// <summary>User settings, stored in %APPDATA%\NotIde\settings.json.</summary>
sealed class Settings
{
    public string? ProjectRoot { get; set; }
    public string? CertoPath { get; set; }
    public string? CodePath { get; set; }
    public string? Target { get; set; }
    public string OutputDir { get; set; } = "out";
    public string ViewMode { get; set; } = "Folders";
    public string ProgramArgs { get; set; } = "";
    public bool Release { get; set; }
    public bool Verbose { get; set; }
    public bool KeepGoing { get; set; }
    public bool AllFiles { get; set; }

    static string FilePath => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "NotIde", "settings.json");

    public static Settings Load()
    {
        try
        {
            if (File.Exists(FilePath))
                return JsonSerializer.Deserialize<Settings>(File.ReadAllText(FilePath)) ?? new Settings();
        }
        catch { /* corrupt settings -> defaults */ }
        return new Settings();
    }

    public void Save()
    {
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(FilePath)!);
            File.WriteAllText(FilePath, JsonSerializer.Serialize(this, new JsonSerializerOptions { WriteIndented = true }));
        }
        catch { /* best effort */ }
    }
}
