using System;
using System.Collections.Generic;
using System.IO;
using Microsoft.Data.Sqlite;
using BC = BCrypt.Net.BCrypt;

namespace NexConsole.Services;

public class LoginResult
{
    public bool Success { get; init; }
    public string Role { get; init; } = "";
    public bool MustChangePassword { get; init; }
    public string Error { get; init; } = "";
}

public class SecurityService
{
    private static readonly string DbPath =
        Path.Combine(AppContext.BaseDirectory, "nex.security.db");

    private static string ConnStr => $"Data Source={DbPath}";

    public SecurityService()
    {
        EnsureSchema();
        EnsureDefaultAdmin();
    }

    private static void EnsureSchema()
    {
        using var conn = Open();
        using var cmd = conn.CreateCommand();
        cmd.CommandText = """
            CREATE TABLE IF NOT EXISTS nex_users (
                id               INTEGER PRIMARY KEY AUTOINCREMENT,
                username         TEXT NOT NULL UNIQUE COLLATE NOCASE,
                password_hash    TEXT NOT NULL,
                role             TEXT NOT NULL DEFAULT 'Viewer',
                must_change_pwd  INTEGER NOT NULL DEFAULT 0,
                created_at       TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS login_history (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                username     TEXT NOT NULL,
                role         TEXT NOT NULL DEFAULT '',
                success      INTEGER NOT NULL,
                failure_reason TEXT,
                logged_at    TEXT NOT NULL DEFAULT (datetime('now'))
            );
            """;
        cmd.ExecuteNonQuery();
    }

    private static void EnsureDefaultAdmin()
    {
        using var conn = Open();
        using var check = conn.CreateCommand();
        check.CommandText = "SELECT COUNT(*) FROM nex_users";
        var count = (long)(check.ExecuteScalar() ?? 0L);
        if (count > 0) return;

        using var insert = conn.CreateCommand();
        insert.CommandText = """
            INSERT INTO nex_users (username, password_hash, role, must_change_pwd)
            VALUES ('admin', $hash, 'Admin', 1)
            """;
        insert.Parameters.AddWithValue("$hash", BC.HashPassword("admin"));
        insert.ExecuteNonQuery();
    }

    public LoginResult Authenticate(string username, string password)
    {
        if (string.IsNullOrWhiteSpace(username) || string.IsNullOrWhiteSpace(password))
            return Fail(username, "Username and password are required.");

        using var conn = Open();
        using var cmd = conn.CreateCommand();
        cmd.CommandText = """
            SELECT password_hash, role, must_change_pwd
            FROM nex_users WHERE username = $u COLLATE NOCASE
            """;
        cmd.Parameters.AddWithValue("$u", username);

        using var reader = cmd.ExecuteReader();
        if (!reader.Read())
            return Fail(username, "Invalid username or password.");

        var hash       = reader.GetString(0);
        var dbRole     = reader.GetString(1);
        var mustChange = reader.GetInt32(2) == 1;

        if (!BC.Verify(password, hash))
            return Fail(username, "Invalid username or password.");

        RecordHistory(conn, username, dbRole, success: true, reason: null);
        return new LoginResult { Success = true, Role = dbRole, MustChangePassword = mustChange };
    }

    public List<string> GetRoles()
    {
        var roles = new List<string>();
        using var conn = Open();
        using var cmd = conn.CreateCommand();
        cmd.CommandText = "SELECT DISTINCT role FROM nex_users ORDER BY role";
        using var reader = cmd.ExecuteReader();
        while (reader.Read()) roles.Add(reader.GetString(0));
        return roles;
    }

    public bool ChangePassword(string username, string newPassword)
    {
        using var conn = Open();
        using var cmd = conn.CreateCommand();
        cmd.CommandText = """
            UPDATE nex_users
            SET password_hash = $hash, must_change_pwd = 0, updated_at = datetime('now')
            WHERE username = $u COLLATE NOCASE
            """;
        cmd.Parameters.AddWithValue("$hash", BC.HashPassword(newPassword));
        cmd.Parameters.AddWithValue("$u", username);
        return cmd.ExecuteNonQuery() > 0;
    }

    private static LoginResult Fail(string username, string reason)
    {
        try
        {
            using var conn = Open();
            RecordHistory(conn, username, "", success: false, reason: reason);
        }
        catch { /* never crash on audit write */ }
        return new LoginResult { Success = false, Error = reason };
    }

    private static void RecordHistory(SqliteConnection conn, string username,
        string role, bool success, string? reason)
    {
        using var cmd = conn.CreateCommand();
        cmd.CommandText = """
            INSERT INTO login_history (username, role, success, failure_reason)
            VALUES ($u, $r, $s, $f)
            """;
        cmd.Parameters.AddWithValue("$u", username);
        cmd.Parameters.AddWithValue("$r", role);
        cmd.Parameters.AddWithValue("$s", success ? 1 : 0);
        cmd.Parameters.AddWithValue("$f", (object?)reason ?? DBNull.Value);
        cmd.ExecuteNonQuery();
    }

    private static SqliteConnection Open()
    {
        var conn = new SqliteConnection(ConnStr);
        conn.Open();
        return conn;
    }
}
