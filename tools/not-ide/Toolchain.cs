using System.Diagnostics;
using System.Text;
using System.Text.RegularExpressions;

namespace NotIde;

enum Sev { Error, Warning, Note }

sealed record Diagnostic(Sev Severity, string Code, string Message, string? File, int Line, int Col);

sealed record Step(string Title, List<string> Args);

sealed record CtoMeta(string? Module, bool HasMain, bool HasTests);

static class Ansi
{
    static readonly Regex Rx = new(@"\x1B\[[0-9;?]*[A-Za-z]", RegexOptions.Compiled);
    public static string Strip(string s) => Rx.Replace(s, "");
}

/// <summary>
/// Turns certo's text output into structured diagnostics. Understands the
/// rustc-style block documented in ERROR-REFERENCE.md
/// ("error[E0100]: msg" followed by "  --> file:line:col") and the common
/// single-line "file.cto:line:col: error: msg" form.
/// </summary>
sealed class DiagnosticParser
{
    static readonly Regex Header = new(@"^\s*(error|warning|note)(?:\[([A-Za-z0-9_]+)\])?:\s*(.*)$",
        RegexOptions.IgnoreCase | RegexOptions.Compiled);
    static readonly Regex Arrow = new(@"^\s*-->\s*(.+?):(\d+)(?::(\d+))?\s*$", RegexOptions.Compiled);
    static readonly Regex Inline = new(@"^(.+?\.cto):(\d+):(\d+):?\s*(error|warning|note)?(?:\[([A-Za-z0-9_]+)\])?:?\s*(.*)$",
        RegexOptions.IgnoreCase | RegexOptions.Compiled);

    // C compiler output for certo's generated C: "<temp>.c:LINE:COL: error: msg". The file is a
    // throwaway temp file, so these become location-less rows (nothing to open) that keep the
    // generated-C line in the message.
    static readonly Regex CcDiag = new(@"^(?:.+?)\.[ch]:(\d+):\d+:\s*(fatal error|error|warning|note):\s*(.*)$",
        RegexOptions.IgnoreCase | RegexOptions.Compiled);

    static readonly Regex Reformat = new(@"^\s*would reformat:\s*(.+?)\s*$", RegexOptions.IgnoreCase | RegexOptions.Compiled);

    (Sev Sev, string Code, string Msg)? _pending;

    public static bool TryLocation(string line, out string file, out int ln, out int col)
    {
        file = ""; ln = 0; col = 0;
        var m = Arrow.Match(line);
        if (!m.Success && !line.TrimStart().StartsWith("-->")) m = Inline.Match(line);
        if (!m.Success) return false;
        file = m.Groups[1].Value.Trim();
        ln = int.Parse(m.Groups[2].Value);
        col = m.Groups[3].Success && m.Groups[3].Value.Length > 0 ? int.Parse(m.Groups[3].Value) : 1;
        return true;
    }

    static Sev ParseSev(string s) => s.ToLowerInvariant() switch
    {
        "error" => Sev.Error,
        "warning" => Sev.Warning,
        _ => Sev.Note,
    };

    readonly Queue<Diagnostic> _queue = new();
    Diagnostic? _extra;

    /// <summary>
    /// Feed one output line; returns a finished diagnostic when one completes. A line can finish two
    /// (a pending header plus the line's own); the extra is queued and comes back on a later Feed or
    /// from <see cref="Flush"/>, so callers should loop on Flush when the stream ends.
    /// </summary>
    public Diagnostic? Feed(string line)
    {
        _extra = null;
        var r = FeedCore(line);
        if (r is not null) _queue.Enqueue(r);
        if (_extra is not null) { _queue.Enqueue(_extra); _extra = null; }
        return _queue.Count > 0 ? _queue.Dequeue() : null;
    }

    Diagnostic? Both(Diagnostic? flushed, Diagnostic d)
    {
        if (flushed is null) return d;
        _extra = d;
        return flushed;
    }

    Diagnostic? FeedCore(string line)
    {
        var fmt = Reformat.Match(line);
        if (fmt.Success)
            return new Diagnostic(Sev.Warning, "fmt", "would reformat", fmt.Groups[1].Value.Trim(), 1, 1);

        var arrow = Arrow.Match(line);
        if (arrow.Success && _pending is { } p)
        {
            _pending = null;
            int col = arrow.Groups[3].Success ? int.Parse(arrow.Groups[3].Value) : 1;
            return new Diagnostic(p.Sev, p.Code, p.Msg, arrow.Groups[1].Value.Trim(),
                int.Parse(arrow.Groups[2].Value), col);
        }

        var cc = CcDiag.Match(line);
        if (cc.Success)
        {
            var flushed = FlushPending();
            var sev = cc.Groups[2].Value.ToLowerInvariant() switch
            {
                "note" => Sev.Note,
                "warning" => Sev.Warning,
                _ => Sev.Error,
            };
            var d = new Diagnostic(sev, "cc", $"{cc.Groups[3].Value.Trim()}  (generated C, line {cc.Groups[1].Value})", null, 0, 0);
            return Both(flushed, d);
        }

        var h = Header.Match(line);
        if (h.Success)
        {
            var flushed = FlushPending();
            _pending = (ParseSev(h.Groups[1].Value), h.Groups[2].Value, h.Groups[3].Value.Trim());
            return flushed;
        }

        if (!line.TrimStart().StartsWith("-->"))
        {
            var inl = Inline.Match(line);
            if (inl.Success)
            {
                var flushed = FlushPending();
                var sev = inl.Groups[4].Success && inl.Groups[4].Value.Length > 0 ? ParseSev(inl.Groups[4].Value) : Sev.Error;
                var d = new Diagnostic(sev, inl.Groups[5].Value, inl.Groups[6].Value.Trim(), inl.Groups[1].Value.Trim(),
                    int.Parse(inl.Groups[2].Value), int.Parse(inl.Groups[3].Value));
                return Both(flushed, d);
            }
        }
        return null;
    }

    /// <summary>Returns queued diagnostics, then any still-pending header; call until it returns null.</summary>
    public Diagnostic? Flush()
    {
        if (_queue.Count > 0) return _queue.Dequeue();
        return FlushPending();
    }

    Diagnostic? FlushPending()
    {
        if (_pending is not { } p) return null;
        _pending = null;
        return new Diagnostic(p.Sev, p.Code, p.Msg, null, 0, 0);
    }
}

/// <summary>Reads the bits of certo.toml the GUI cares about.</summary>
sealed class ProjectInfo
{
    public string Root { get; }
    public string? ManifestPath { get; private set; }
    public string Name { get; private set; }
    public string? Entry { get; private set; }

    ProjectInfo(string root) { Root = root; Name = Path.GetFileName(root.TrimEnd('\\', '/')); }

    static readonly Regex Section = new(@"^\s*\[([^\]]+)\]", RegexOptions.Compiled);
    static readonly Regex KeyVal = new(@"^\s*([A-Za-z0-9_-]+)\s*=\s*""([^""]*)""", RegexOptions.Compiled);

    public static ProjectInfo Load(string root)
    {
        var info = new ProjectInfo(root);
        var path = Path.Combine(root, "certo.toml");
        if (!File.Exists(path)) return info;
        info.ManifestPath = path;
        string section = "";
        try
        {
            foreach (var line in File.ReadLines(path))
            {
                var s = Section.Match(line);
                if (s.Success) { section = s.Groups[1].Value.Trim(); continue; }
                var kv = KeyVal.Match(line);
                if (!kv.Success) continue;
                if (section == "project" && kv.Groups[1].Value == "name") info.Name = kv.Groups[2].Value;
                if (section == "build" && kv.Groups[1].Value == "entry") info.Entry = kv.Groups[2].Value;
            }
        }
        catch { /* unreadable manifest -> treat as absent */ }
        return info;
    }

    public string? EntryFullPath =>
        Entry is null ? null : Path.GetFullPath(Path.Combine(Root, Entry.Replace('/', Path.DirectorySeparatorChar)));
}

/// <summary>Cheap per-file scan of a .cto source, cached by write time.</summary>
static class Scanner
{
    static readonly Regex Module = new(@"^\s*module\s+([A-Za-z_][\w.]*)", RegexOptions.Multiline | RegexOptions.Compiled);
    static readonly Regex Main = new(@"^\s*(?:pub\s+)?fn\s+main\s*\(", RegexOptions.Multiline | RegexOptions.Compiled);
    static readonly Regex Test = new(@"^\s*test\s+""", RegexOptions.Multiline | RegexOptions.Compiled);
    static readonly Dictionary<string, (DateTime, long, CtoMeta)> Cache = new(StringComparer.OrdinalIgnoreCase);

    public static CtoMeta Meta(string path)
    {
        try
        {
            var fi = new FileInfo(path);
            if (Cache.TryGetValue(path, out var c) && c.Item1 == fi.LastWriteTimeUtc && c.Item2 == fi.Length)
                return c.Item3;
            string text = fi.Length < 2_000_000 ? File.ReadAllText(path) : "";
            var m = Module.Match(text);
            var meta = new CtoMeta(m.Success ? m.Groups[1].Value : null, Main.IsMatch(text), Test.IsMatch(text));
            Cache[path] = (fi.LastWriteTimeUtc, fi.Length, meta);
            return meta;
        }
        catch { return new CtoMeta(null, false, false); }
    }
}

/// <summary>Finds certo and VS Code on this machine.</summary>
static class Locator
{
    public static string? FindCerto(string? configured, string? root)
    {
        var candidates = new List<string?> { configured };
        if (root != null)
            foreach (var rel in new[] { @"dist\certo.exe", @"target\release\certo.exe", @"target\debug\certo.exe", "certo.exe" })
                candidates.Add(Path.Combine(root, rel));
        foreach (var c in candidates)
            if (!string.IsNullOrWhiteSpace(c) && File.Exists(c)) return c;
        return OnPath("certo.exe");
    }

    public static string? FindCode(string? configured)
    {
        if (!string.IsNullOrWhiteSpace(configured) && File.Exists(configured)) return configured;
        string local = Environment.GetEnvironmentVariable("LOCALAPPDATA") ?? "";
        string pf = Environment.GetEnvironmentVariable("ProgramFiles") ?? "";
        foreach (var c in new[]
        {
            Path.Combine(local, "Programs", "Microsoft VS Code", "Code.exe"),
            Path.Combine(pf, "Microsoft VS Code", "Code.exe"),
        })
            if (File.Exists(c)) return c;
        return OnPath("code.cmd") ?? OnPath("code.exe");
    }

    static string? OnPath(string file)
    {
        foreach (var dir in (Environment.GetEnvironmentVariable("PATH") ?? "").Split(Path.PathSeparator))
        {
            try
            {
                var p = Path.Combine(dir.Trim().Trim('"'), file);
                if (File.Exists(p)) return p;
            }
            catch { }
        }
        return null;
    }
}

/// <summary>Runs one process at a time, streaming output lines.</summary>
sealed class Runner
{
    public async Task<int> RunAsync(string exe, IEnumerable<string> args, string cwd,
        Action<string> onLine, CancellationToken ct)
    {
        var psi = new ProcessStartInfo(exe)
        {
            WorkingDirectory = cwd,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            CreateNoWindow = true,
            StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8,
        };
        foreach (var a in args) psi.ArgumentList.Add(a);
        psi.Environment["NO_COLOR"] = "1";

        using var p = new Process { StartInfo = psi };
        p.OutputDataReceived += (_, e) => { if (e.Data != null) onLine(e.Data); };
        p.ErrorDataReceived += (_, e) => { if (e.Data != null) onLine(e.Data); };
        p.Start();
        p.BeginOutputReadLine();
        p.BeginErrorReadLine();
        using var reg = ct.Register(() =>
        {
            try { if (!p.HasExited) p.Kill(entireProcessTree: true); } catch { }
        });
        await p.WaitForExitAsync(CancellationToken.None);
        return ct.IsCancellationRequested ? -1 : p.ExitCode;
    }
}

static class ArgSplitter
{
    /// <summary>Splits "a b 'c d'" style program arguments, honouring double quotes.</summary>
    public static List<string> Split(string s)
    {
        var res = new List<string>();
        var sb = new StringBuilder();
        bool inQ = false, any = false;
        foreach (char ch in s)
        {
            if (ch == '"') { inQ = !inQ; any = true; }
            else if (char.IsWhiteSpace(ch) && !inQ)
            {
                if (any || sb.Length > 0) { res.Add(sb.ToString()); sb.Clear(); any = false; }
            }
            else sb.Append(ch);
        }
        if (any || sb.Length > 0) res.Add(sb.ToString());
        return res;
    }
}
