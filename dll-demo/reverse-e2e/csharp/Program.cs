using System;
using System.Runtime.InteropServices;

// End-to-end: C#  ->  Certo DLL (strutil.dll)  ->  Rust DLL (strrev.dll)
//
// Certo's `pub fn reverseString(s: Text): Reversed` exports as `certo_reverse_string`
// and returns the record `Reversed { reversed: Text, length: Int }` by value, which
// is the C struct { const char* reversed; int64_t length; }.
//   Certo Text -> const char*  -> C# string (in) / IntPtr (out)
//   Certo Int  -> int64_t      -> C# long
internal static class Program
{
    [StructLayout(LayoutKind.Sequential)]
    private struct Reversed
    {
        public IntPtr reversed;   // const char* (UTF-8, owned by the native side)
        public long length;       // int64_t
    }

    [DllImport("strutil.dll", EntryPoint = "certo_reverse_string", CallingConvention = CallingConvention.Cdecl)]
    private static extern Reversed ReverseString([MarshalAs(UnmanagedType.LPUTF8Str)] string s);

    private static void Main()
    {
        const string input = "abcdef";
        Reversed r = ReverseString(input);
        string reversed = Marshal.PtrToStringUTF8(r.reversed) ?? "<null>";

        Console.WriteLine($"input:    {input}");
        Console.WriteLine($"reversed: {reversed}");
        Console.WriteLine($"length:   {r.length}");
    }
}
