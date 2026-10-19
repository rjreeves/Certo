using System.Text.Json;
using System.Text.Json.Serialization;

namespace NotIde;

enum RunResult { Succeeded, Warnings, Failed, Stopped }

/// <summary>One finished command: what ran, on what, how it ended, and how long it took.</summary>
sealed record RunRecord(
    DateTime When, string Project, string Command, string Target,
    RunResult Result, int Errors, int Warnings, double Seconds);

/// <summary>
/// The list of past runs, newest first, kept in %APPDATA%\NotIde\history.json and capped so it
/// can't grow without bound. UI-free so it stays unit-testable.
/// </summary>
sealed class RunHistory
{
    public const int Max = 200;

    static readonly JsonSerializerOptions Json = new()
    {
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter() },
    };

    readonly string _path;
    public List<RunRecord> Items { get; private set; } = new();

    public RunHistory(string path) { _path = path; }

    public static string DefaultPath => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "NotIde", "history.json");

    public static RunHistory Load(string? path = null)
    {
        var h = new RunHistory(path ?? DefaultPath);
        try
        {
            if (File.Exists(h._path))
                h.Items = JsonSerializer.Deserialize<List<RunRecord>>(File.ReadAllText(h._path), Json) ?? new();
        }
        catch { /* corrupt history -> start empty */ }
        return h;
    }

    public void Add(RunRecord r)
    {
        Items.Insert(0, r);
        if (Items.Count > Max) Items.RemoveRange(Max, Items.Count - Max);
        Save();
    }

    public void Clear()
    {
        Items.Clear();
        Save();
    }

    void Save()
    {
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(_path)!);
            File.WriteAllText(_path, JsonSerializer.Serialize(Items, Json));
        }
        catch { /* best effort */ }
    }
}
