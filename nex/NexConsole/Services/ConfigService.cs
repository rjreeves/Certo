using System;
using System.IO;
using System.Text.Json;
using NexConsole.Models;

namespace NexConsole.Services;

public class ConfigService
{
    private static readonly string ConfigPath =
        Path.Combine(AppContext.BaseDirectory, "nex.config.json");

    private static readonly string LoginCachePath =
        Path.Combine(AppContext.BaseDirectory, "nex.login.json");

    private static readonly JsonSerializerOptions ReadOptions = new()
    {
        ReadCommentHandling = JsonCommentHandling.Skip,
        AllowTrailingCommas = true,
        PropertyNameCaseInsensitive = true,
    };

    private static readonly JsonSerializerOptions WriteOptions = new()
    {
        WriteIndented = true,
    };

    public NexConfig Config { get; }

    public ConfigService()
    {
        Config = Load();
        // Merge saved login from separate cache file (keeps JSONC config intact)
        Config.Login = LoadLogin();
    }

    private static NexConfig Load()
    {
        if (!File.Exists(ConfigPath))
            return new NexConfig();
        try
        {
            return JsonSerializer.Deserialize<NexConfig>(File.ReadAllText(ConfigPath), ReadOptions)
                   ?? new NexConfig();
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"[ConfigService] Failed to load {ConfigPath}: {ex.Message}");
            return new NexConfig();
        }
    }

    private static SavedLogin? LoadLogin()
    {
        if (!File.Exists(LoginCachePath))
            return null;
        try
        {
            return JsonSerializer.Deserialize<SavedLogin>(File.ReadAllText(LoginCachePath), ReadOptions);
        }
        catch { return null; }
    }

    public void SaveLogin(string username, string role)
    {
        try
        {
            File.WriteAllText(LoginCachePath,
                JsonSerializer.Serialize(new SavedLogin { Username = username, Role = role }, WriteOptions));
            Config.Login = new SavedLogin { Username = username, Role = role };
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"[ConfigService] Failed to save login: {ex.Message}");
        }
    }

    public void ClearLogin()
    {
        try { File.Delete(LoginCachePath); } catch { }
        Config.Login = null;
    }
}
