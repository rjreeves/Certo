using System;
using System.Diagnostics;
using System.Threading.Tasks;
using Avalonia.Media;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Npgsql;

namespace NexConsole.ViewModels;

public partial class ConnectionProbeViewModel : ViewModelBase
{
    private readonly string _connectionString;

    public string ConnectionName { get; }

    [ObservableProperty] private string _version    = "—";
    [ObservableProperty] private string _selectOne  = "—";
    [ObservableProperty] private string _schema     = "—";
    [ObservableProperty] private string _error      = "";
    [ObservableProperty] private bool   _isRunning;
    [ObservableProperty] private bool   _hasError;

    [ObservableProperty] private IBrush _schemaColor =
        new SolidColorBrush(Color.Parse("#555555"));

    public ConnectionProbeViewModel(string name, string connectionString)
    {
        ConnectionName    = name;
        _connectionString = connectionString;
    }

    [RelayCommand(CanExecute = nameof(CanRun))]
    private async Task Run()
    {
        IsRunning = true;
        HasError  = false;
        Error     = "";
        Version   = "checking…";
        SelectOne = "checking…";
        Schema    = "checking…";
        SchemaColor = new SolidColorBrush(Color.Parse("#555555"));

        try
        {
            await using var conn = new NpgsqlConnection(_connectionString);
            await conn.OpenAsync();

            // Version
            await using (var cmd = conn.CreateCommand())
            {
                cmd.CommandText = "SELECT version()";
                Version = (string)(await cmd.ExecuteScalarAsync() ?? "unknown");
            }

            // SELECT 1 latency
            await using (var cmd = conn.CreateCommand())
            {
                cmd.CommandText = "SELECT 1";
                var sw = Stopwatch.StartNew();
                await cmd.ExecuteScalarAsync();
                sw.Stop();
                SelectOne = $"{sw.ElapsedMilliseconds} ms";
            }

            // Schema check
            await using (var cmd = conn.CreateCommand())
            {
                cmd.CommandText = """
                    SELECT COUNT(*) FROM information_schema.schemata
                    WHERE schema_name = 'fireworks'
                    """;
                var count = Convert.ToInt64(await cmd.ExecuteScalarAsync());
                if (count > 0)
                {
                    Schema      = "fireworks — found";
                    SchemaColor = new SolidColorBrush(Color.Parse("#4caf7d"));
                }
                else
                {
                    Schema      = "fireworks — not found";
                    SchemaColor = new SolidColorBrush(Color.Parse("#e25c5c"));
                }
            }
        }
        catch (Exception ex)
        {
            HasError  = true;
            Error     = ex.Message;
            Version   = "—";
            SelectOne = "—";
            Schema    = "—";
            SchemaColor = new SolidColorBrush(Color.Parse("#555555"));
        }
        finally
        {
            IsRunning = false;
        }
    }

    private bool CanRun() => !IsRunning;
}
