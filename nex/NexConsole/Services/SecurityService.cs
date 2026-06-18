using System;
using System.Collections.Generic;
using System.IO;
using System.Threading.Tasks;
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

public class UserRecord
{
    public int    Id             { get; init; }
    public string Username       { get; init; } = "";
    public string Role           { get; init; } = "";
    public bool   IsEnabled      { get; init; }
    public bool   MustChangePwd  { get; init; }
    public string CreatedAt      { get; init; } = "";
    public string UpdatedAt      { get; init; } = "";
}

public class SecurityService
{
    // Shared instance — background-initialised once, reused everywhere
    public static readonly Task<SecurityService> InitAsync =
        Task.Run(() => new SecurityService());

    private static readonly string DbPath =
        Path.Combine(AppContext.BaseDirectory, "nex.security.db");

    private static string ConnStr => $"Data Source={DbPath}";

    private SecurityService()
    {
        EnsureSchema();
        EnsureDefaultUsers();
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
                role             TEXT NOT NULL DEFAULT 'Application',
                is_enabled       INTEGER NOT NULL DEFAULT 1,
                must_change_pwd  INTEGER NOT NULL DEFAULT 0,
                created_at       TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS login_history (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                username       TEXT NOT NULL,
                role           TEXT NOT NULL DEFAULT '',
                success        INTEGER NOT NULL,
                failure_reason TEXT,
                logged_at      TEXT NOT NULL DEFAULT (datetime('now'))
            );
            """;
        cmd.ExecuteNonQuery();

        // Migrate existing DB that may be missing is_enabled
        try
        {
            using var alter = conn.CreateCommand();
            alter.CommandText = "ALTER TABLE nex_users ADD COLUMN is_enabled INTEGER NOT NULL DEFAULT 1";
            alter.ExecuteNonQuery();
        }
        catch { /* column already exists — safe to ignore */ }
    }

    private static void EnsureDefaultUsers()
    {
        using var conn = Open();
        using var check = conn.CreateCommand();
        check.CommandText = "SELECT COUNT(*) FROM nex_users";
        if ((long)(check.ExecuteScalar() ?? 0L) > 0) return;

        using var cmd = conn.CreateCommand();
        cmd.CommandText = """
            INSERT INTO nex_users (username, password_hash, role, must_change_pwd) VALUES
                ('owner', $ownerHash, 'Owner',         0),
                ('admin', $adminHash, 'Administrator', 1)
            """;
        cmd.Parameters.AddWithValue("$ownerHash", BC.HashPassword("123456", workFactor: 10));
        cmd.Parameters.AddWithValue("$adminHash", BC.HashPassword("Admin",  workFactor: 10));
        cmd.ExecuteNonQuery();
    }

    // ── Authentication ────────────────────────────────────────────────────────

    public LoginResult Authenticate(string username, string password)
    {
        if (string.IsNullOrWhiteSpace(username) || string.IsNullOrWhiteSpace(password))
            return new LoginResult { Success = false, Error = "Username and password are required." };

        using var conn = Open();

        string? hash    = null;
        string  dbRole  = "";
        bool mustChange = false;
        bool enabled    = true;

        using (var cmd = conn.CreateCommand())
        {
            cmd.CommandText = """
                SELECT password_hash, role, must_change_pwd, is_enabled
                FROM nex_users WHERE username = $u COLLATE NOCASE
                """;
            cmd.Parameters.AddWithValue("$u", username);

            using var reader = cmd.ExecuteReader();
            if (reader.Read())
            {
                hash       = reader.GetString(0);
                dbRole     = reader.GetString(1);
                mustChange = reader.GetInt32(2) == 1;
                enabled    = reader.GetInt32(3) == 1;
            }
        } // reader + cmd closed before any further DB or BCrypt work

        if (hash is null)
        {
            RecordHistory(conn, username, "", success: false, reason: "Invalid username or password.");
            return new LoginResult { Success = false, Error = "Invalid username or password." };
        }

        if (!enabled)
        {
            RecordHistory(conn, username, dbRole, success: false, reason: "Account disabled.");
            return new LoginResult { Success = false, Error = "This account is disabled." };
        }

        // BCrypt runs after the reader is closed — no lock contention
        if (!BC.Verify(password, hash))
        {
            RecordHistory(conn, username, dbRole, success: false, reason: "Invalid password.");
            return new LoginResult { Success = false, Error = "Invalid username or password." };
        }

        RecordHistory(conn, username, dbRole, success: true, reason: null);
        return new LoginResult { Success = true, Role = dbRole, MustChangePassword = mustChange };
    }

    // ── Password ──────────────────────────────────────────────────────────────

    public bool ChangePassword(string username, string newPassword)
    {
        using var conn = Open();
        using var cmd  = conn.CreateCommand();
        cmd.CommandText = """
            UPDATE nex_users
            SET password_hash = $hash, must_change_pwd = 0, updated_at = datetime('now')
            WHERE username = $u COLLATE NOCASE
            """;
        cmd.Parameters.AddWithValue("$hash", BC.HashPassword(newPassword, workFactor: 10));
        cmd.Parameters.AddWithValue("$u", username);
        return cmd.ExecuteNonQuery() > 0;
    }

    // ── User management ───────────────────────────────────────────────────────

    public List<UserRecord> GetUsers()
    {
        var list = new List<UserRecord>();
        using var conn = Open();
        using var cmd  = conn.CreateCommand();
        cmd.CommandText = """
            SELECT id, username, role, is_enabled, must_change_pwd, created_at, updated_at
            FROM nex_users ORDER BY username COLLATE NOCASE
            """;
        using var reader = cmd.ExecuteReader();
        while (reader.Read())
            list.Add(new UserRecord
            {
                Id            = reader.GetInt32(0),
                Username      = reader.GetString(1),
                Role          = reader.GetString(2),
                IsEnabled     = reader.GetInt32(3) == 1,
                MustChangePwd = reader.GetInt32(4) == 1,
                CreatedAt     = reader.GetString(5),
                UpdatedAt     = reader.GetString(6),
            });
        return list;
    }

    public (bool ok, string error) AddUser(string username, string password, string role)
    {
        if (string.IsNullOrWhiteSpace(username)) return (false, "Username is required.");
        if (password.Length < 6)                 return (false, "Password must be at least 6 characters.");

        using var conn = Open();
        try
        {
            using var cmd = conn.CreateCommand();
            cmd.CommandText = """
                INSERT INTO nex_users (username, password_hash, role, must_change_pwd)
                VALUES ($u, $hash, $role, 1)
                """;
            cmd.Parameters.AddWithValue("$u",    username.Trim());
            cmd.Parameters.AddWithValue("$hash", BC.HashPassword(password, workFactor: 10));
            cmd.Parameters.AddWithValue("$role", role);
            cmd.ExecuteNonQuery();
            return (true, "");
        }
        catch (SqliteException ex) when (ex.Message.Contains("UNIQUE"))
        {
            return (false, $"Username '{username}' already exists.");
        }
    }

    public bool UpdateUser(int id, string role, bool mustChangePwd)
    {
        using var conn = Open();
        using var cmd  = conn.CreateCommand();
        cmd.CommandText = """
            UPDATE nex_users
            SET role = $role, must_change_pwd = $mcp, updated_at = datetime('now')
            WHERE id = $id
            """;
        cmd.Parameters.AddWithValue("$role", role);
        cmd.Parameters.AddWithValue("$mcp",  mustChangePwd ? 1 : 0);
        cmd.Parameters.AddWithValue("$id",   id);
        return cmd.ExecuteNonQuery() > 0;
    }

    public bool SetEnabled(int id, bool enabled)
    {
        using var conn = Open();
        using var cmd  = conn.CreateCommand();
        cmd.CommandText = """
            UPDATE nex_users
            SET is_enabled = $e, updated_at = datetime('now')
            WHERE id = $id
            """;
        cmd.Parameters.AddWithValue("$e",  enabled ? 1 : 0);
        cmd.Parameters.AddWithValue("$id", id);
        return cmd.ExecuteNonQuery() > 0;
    }

    public List<string> GetRoles()
    {
        var roles = new List<string>();
        using var conn = Open();
        using var cmd  = conn.CreateCommand();
        cmd.CommandText = "SELECT DISTINCT role FROM nex_users ORDER BY role";
        using var reader = cmd.ExecuteReader();
        while (reader.Read()) roles.Add(reader.GetString(0));
        return roles;
    }

    // ── Internals ─────────────────────────────────────────────────────────────

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
