using Npgsql;

namespace Oe.Infrastructure.Database;

public sealed class DbConnectionFactory(string connectionString)
{
    public NpgsqlConnection Create() => new(connectionString);
}
