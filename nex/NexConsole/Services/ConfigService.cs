using System;
using System.IO;
using System.Security.Cryptography;
using System.Text;
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

    public void SaveLogin(string username, string role, string password)
    {
        try
        {
            var saved = new SavedLogin
            {
                Username          = username,
                Role              = role,
                ProtectedPassword = ProtectPassword(password),
            };
            File.WriteAllText(LoginCachePath, JsonSerializer.Serialize(saved, WriteOptions));
            Config.Login = saved;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"[ConfigService] Failed to save login: {ex.Message}");
        }
    }

    // Returns null if decryption fails (e.g. different Windows user or corrupted cache)
    public static string? UnprotectPassword(string? protectedBase64)
    {
        if (string.IsNullOrEmpty(protectedBase64)) return null;
        try
        {
            var cipher  = Convert.FromBase64String(protectedBase64);
            var plain   = ProtectedData.Unprotect(cipher, null, DataProtectionScope.CurrentUser);
            return Encoding.UTF8.GetString(plain);
        }
        catch { return null; }
    }

    private static string? ProtectPassword(string password)
    {
        try
        {
            var plain  = Encoding.UTF8.GetBytes(password);
            var cipher = ProtectedData.Protect(plain, null, DataProtectionScope.CurrentUser);
            return Convert.ToBase64String(cipher);
        }
        catch { return null; }
    }

    public void ClearLogin()
    {
        try { File.Delete(LoginCachePath); } catch { }
        Config.Login = null;
    }
}
