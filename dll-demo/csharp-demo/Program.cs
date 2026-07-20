using System.Runtime.InteropServices;

// C# consumer: P/Invokes the Certo-built add.dll. No header or import lib needed —
// only the exported symbol name and a matching signature.
//   Certo `pub fn add` -> C symbol `certo_add` (certo_ prefix + snake_case)
//   Certo `Int`        -> int64_t -> C# `long`
internal static class Program
{
    [DllImport("add.dll", EntryPoint = "certo_add", CallingConvention = CallingConvention.Cdecl)]
    private static extern long CertoAdd(long a, long b);

    private static void Main()
    {
        long result = CertoAdd(2, 3);
        System.Console.WriteLine($"certo_add(2, 3) = {result}");
    }
}
