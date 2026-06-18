using System.Collections.Generic;

namespace NexConsole.Models;

public class NexConfig
{
    public List<ConnectionEntry> Connections { get; set; } = [];
    public SavedLogin? Login { get; set; }
}

public class SavedLogin
{
    public string Username { get; set; } = "";
    public string Role { get; set; } = "";
}

public class ConnectionEntry
{
    public string Name { get; set; } = "";
    public string ConnectionString { get; set; } = "";
}
