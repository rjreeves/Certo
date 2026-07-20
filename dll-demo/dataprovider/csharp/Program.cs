using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text.Json;

// C# client of the Certo data-provider DLL.
//
//   C#  ->  dataprovider.dll (Certo)  ->  libpq  ->  PostgreSQL
//
// certo_query_db(connStr, sql) returns a malloc'd UTF-8 JSON string (const char*).
// We copy it into a managed string, then hand the native buffer back to
// certo_free so the provider's allocator releases it.
internal static class Program
{
    private const string Dll = "dataprovider.dll";

    [DllImport(Dll, EntryPoint = "certo_query_db", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr QueryDb(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string connStr,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string sql);

    [DllImport(Dll, EntryPoint = "certo_free", CallingConvention = CallingConvention.Cdecl)]
    private static extern void CertoFree(IntPtr p);

    private static string Query(string connStr, string sql)
    {
        IntPtr ptr = QueryDb(connStr, sql);
        try { return Marshal.PtrToStringUTF8(ptr) ?? "[]"; }
        finally { CertoFree(ptr); }            // release the native buffer
    }

    private static void Main()
    {
        string json = Query(
            "postgresql://postgres@localhost:5432/postgres",
            "SELECT id, name, email FROM certo_demo_users ORDER BY id");

        // Deserialize the rowset into typed objects.
        var rows = JsonSerializer.Deserialize<List<Dictionary<string, string?>>>(json)!;

        Console.WriteLine($"{rows.Count} rows from the Certo data provider:\n");
        Console.WriteLine($"{"id",-4} {"name",-18} email");
        Console.WriteLine(new string('-', 45));
        foreach (var r in rows)
            Console.WriteLine($"{r["id"],-4} {r["name"],-18} {r["email"] ?? "(null)"}");
    }
}
