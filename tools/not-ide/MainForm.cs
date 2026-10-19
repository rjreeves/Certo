using System.Diagnostics;
using System.Drawing;

namespace NotIde;

sealed record NodeInfo(string Key, string? Path, bool IsDir);

/// <summary>not-ide: a small GUI that organises a Certo project and drives the certo toolchain.</summary>
sealed class MainForm : Form
{
    // ── state ────────────────────────────────────────────────────────────
    readonly Settings _s = Settings.Load();
    readonly Runner _runner = new();
    ProjectInfo? _info;
    string? _root;
    string? _target;
    CancellationTokenSource? _cts;
    bool _busy;
    readonly System.Diagnostics.Stopwatch _clock = new();
    readonly RunHistory _history = RunHistory.Load();
    int _errors, _warnings;
    FileSystemWatcher? _watcher;
    SplitContainer _mainSplit = null!, _rightSplit = null!;

    static readonly HashSet<string> IgnoredDirs = new(StringComparer.OrdinalIgnoreCase)
        { ".git", "target", "node_modules", "bin", "obj", ".vs", "dist", "out" };
    static readonly HashSet<string> HiddenExts = new(StringComparer.OrdinalIgnoreCase)
        { ".exe", ".dll", ".lib", ".obj", ".pdb", ".o", ".zip", ".docx", ".png", ".jpg", ".ico", ".lock" };

    // ── controls ─────────────────────────────────────────────────────────
    readonly TreeView _tree = new();
    readonly ComboBox _view = new();
    readonly TextBox _filter = new();
    readonly ListView _diag = new();
    readonly RichTextBox _log = new();
    readonly Label _diagHeader = new();
    readonly ToolStripLabel _targetLabel = new();
    readonly ToolStripStatusLabel _stState = new() { Text = "Ready" };
    readonly ToolStripStatusLabel _stCerto = new() { Spring = true, TextAlign = ContentAlignment.MiddleRight };
    readonly ToolStripStatusLabel _stTime = new();
    readonly List<ToolStripItem> _cmdItems = new();
    ToolStripButton _stopBtn = null!;
    ToolStripMenuItem _releaseBtn = null!, _verboseBtn = null!, _keepBtn = null!;
    ToolStripComboBox _scope = null!;
    ToolStripTextBox _args = null!;
    readonly System.Windows.Forms.Timer _tick = new() { Interval = 250 };
    readonly System.Windows.Forms.Timer _refreshDebounce = new() { Interval = 600 };
    readonly System.Windows.Forms.Timer _filterDebounce = new() { Interval = 250 };

    public MainForm(string? startPath)
    {
        Text = "not-ide — Certo dev GUI";
        Width = 1280; Height = 800;
        MinimumSize = new Size(900, 560);
        StartPosition = FormStartPosition.CenterScreen;
        Font = new Font("Segoe UI", 9f);
        KeyPreview = true;

        BuildUi();

        _tick.Tick += (_, _) => { if (_busy) _stTime.Text = FormatElapsed(_clock.Elapsed); };
        _refreshDebounce.Tick += (_, _) => { _refreshDebounce.Stop(); RefreshTree(); };
        _filterDebounce.Tick += (_, _) => { _filterDebounce.Stop(); RefreshTree(); };

        Load += async (_, _) =>
        {
            // Splitter distances can only be set once the form has its real size.
            try { _mainSplit.SplitterDistance = 340; _rightSplit.SplitterDistance = Math.Max(120, _rightSplit.Height / 3); } catch { }
            var path = startPath ?? _s.ProjectRoot;
            if (!string.IsNullOrEmpty(path) && Directory.Exists(path)) OpenProject(path);
            else Append("Open a Certo project folder (File ▸ Open…, Ctrl+O) to begin.\n", Color.Gray);
            await ShowCertoVersionAsync();
        };
        FormClosing += (_, _) =>
        {
            _cts?.Cancel();
            _s.Release = _releaseBtn.Checked; _s.Verbose = _verboseBtn.Checked; _s.KeepGoing = _keepBtn.Checked;
            _s.AllFiles = _scope.SelectedIndex == 1; _s.ProgramArgs = _args.Text;
            _s.ViewMode = _view.SelectedItem?.ToString() ?? "Folders";
            _s.Target = _target;
            _s.Save();
        };
    }

    // ── UI construction ──────────────────────────────────────────────────
    void BuildUi()
    {
        // Status bar
        var status = new StatusStrip();
        status.Items.AddRange(new ToolStripItem[] { _stState, _stCerto, _stTime });
        Controls.Add(status);

        // Main split: tree | (problems / output)
        var main = new SplitContainer { Dock = DockStyle.Fill, FixedPanel = FixedPanel.Panel1 };
        _mainSplit = main;
        Controls.Add(main);
        main.BringToFront();

        // Left: view + filter + tree
        var leftTop = new TableLayoutPanel { Dock = DockStyle.Top, AutoSize = true, AutoSizeMode = AutoSizeMode.GrowAndShrink, ColumnCount = 2, RowCount = 1, Padding = new Padding(6, 6, 6, 6) };
        leftTop.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        leftTop.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
        _view.DropDownStyle = ComboBoxStyle.DropDownList;
        _view.Items.AddRange(new object[] { "Folders", "By module", "By type" });
        _view.SelectedItem = _s.ViewMode; if (_view.SelectedIndex < 0) _view.SelectedIndex = 0;
        _view.Dock = DockStyle.Fill; _view.Margin = new Padding(0, 0, 6, 0);
        _view.SelectedIndexChanged += (_, _) => RefreshTree();
        _filter.PlaceholderText = "Filter files…";
        _filter.Dock = DockStyle.Fill;
        _filter.TextChanged += (_, _) => { _filterDebounce.Stop(); _filterDebounce.Start(); };
        leftTop.Controls.Add(_view, 0, 0);
        leftTop.Controls.Add(_filter, 1, 0);

        _tree.Dock = DockStyle.Fill;
        _tree.HideSelection = false;
        _tree.ShowNodeToolTips = true;
        _tree.AfterSelect += (_, e) =>
        {
            if (e.Node?.Tag is NodeInfo { IsDir: false, Path: { } p } && p.EndsWith(".cto", StringComparison.OrdinalIgnoreCase))
                SetTarget(p);
        };
        _tree.NodeMouseDoubleClick += (_, e) =>
        {
            if (e.Node.Tag is NodeInfo { IsDir: false, Path: { } p }) OpenInCode(p);
        };
        _tree.NodeMouseClick += (_, e) => { if (e.Button == MouseButtons.Right) _tree.SelectedNode = e.Node; };
        _tree.ContextMenuStrip = BuildTreeMenu();
        main.Panel1.Controls.Add(_tree);
        main.Panel1.Controls.Add(leftTop);

        // Right: problems on top, output below
        var right = new SplitContainer { Dock = DockStyle.Fill, Orientation = Orientation.Horizontal };
        _rightSplit = right;
        main.Panel2.Controls.Add(right);

        _diagHeader.Text = "Problems"; _diagHeader.Dock = DockStyle.Top; _diagHeader.AutoSize = true;
        _diagHeader.Padding = new Padding(6, 6, 0, 6); _diagHeader.Font = new Font(Font, FontStyle.Bold);
        _diag.Dock = DockStyle.Fill; _diag.View = View.Details; _diag.FullRowSelect = true; _diag.HideSelection = false;
        _diag.Columns.Add("Severity", 80); _diag.Columns.Add("Code", 70); _diag.Columns.Add("Message", 520); _diag.Columns.Add("Location", 260);
        _diag.DoubleClick += (_, _) =>
        {
            if (_diag.SelectedItems.Count > 0 && _diag.SelectedItems[0].Tag is Diagnostic { File: { } f } d)
                OpenLocation(f, d.Line, d.Col);
        };
        right.Panel1.Controls.Add(_diag);
        right.Panel1.Controls.Add(_diagHeader);

        _log.Dock = DockStyle.Fill; _log.ReadOnly = true; _log.WordWrap = false;
        _log.BackColor = Color.FromArgb(30, 30, 30); _log.ForeColor = Color.FromArgb(212, 212, 212);
        _log.Font = new Font("Consolas", 10f); _log.BorderStyle = BorderStyle.None;
        _log.DoubleClick += (_, _) => OpenLogLine();
        var logHeader = new Label
        {
            Text = "Output  (double-click a file:line to open it in VS Code)", Dock = DockStyle.Top, AutoSize = true,
            Padding = new Padding(6, 6, 0, 6), Font = new Font(Font, FontStyle.Bold),
        };
        right.Panel2.Controls.Add(_log);
        right.Panel2.Controls.Add(logHeader);

        // Command strip then project strip (added last = docked on top)
        Controls.Add(BuildProjectStrip());
        Controls.Add(BuildCommandStrip());
        var menu = BuildMenu();
        Controls.Add(menu);
        MainMenuStrip = menu;
    }

    MenuStrip BuildMenu()
    {
        var ms = new MenuStrip();
        var file = new ToolStripMenuItem("&File");
        file.DropDownItems.Add(new ToolStripMenuItem("&Open project…", null, (_, _) => PickProject(), Keys.Control | Keys.O));
        var recent = new ToolStripMenuItem("Open &recent");
        file.DropDownOpening += (_, _) => FillRecentMenu(recent);
        file.DropDownItems.Add(recent);
        file.DropDownItems.Add(new ToolStripSeparator());
        file.DropDownItems.Add(new ToolStripMenuItem("Open folder in &VS Code", null, (_, _) => OpenFolderInCode()));
        file.DropDownItems.Add(new ToolStripMenuItem("Open &certo.toml", null, (_, _) =>
        {
            if (_info?.ManifestPath is { } m) OpenInCode(m); else Warn("This project has no certo.toml.");
        }));
        file.DropDownItems.Add(new ToolStripMenuItem("&Reload tree", null, (_, _) => RefreshTree()));
        file.DropDownItems.Add(new ToolStripSeparator());
        file.DropDownItems.Add(new ToolStripMenuItem("E&xit", null, (_, _) => Close()));
        ms.Items.Add(file);

        var view = new ToolStripMenuItem("&View");
        view.DropDownItems.Add(new ToolStripMenuItem("Run &history…", null, (_, _) => ShowHistory(), Keys.Control | Keys.H));
        ms.Items.Add(view);
        return ms;
    }

    /// <summary>Refills File ▸ Open recent from the saved list (rebuilt on each open, so it's always current).</summary>
    void FillRecentMenu(ToolStripMenuItem menu)
    {
        menu.DropDownItems.Clear();
        var shown = _s.RecentProjects
            .Where(p => Directory.Exists(p) && !string.Equals(p, _root, StringComparison.OrdinalIgnoreCase))
            .ToList();
        if (shown.Count == 0)
        {
            menu.DropDownItems.Add(new ToolStripMenuItem("(no recent projects)") { Enabled = false });
            return;
        }
        for (int i = 0; i < shown.Count; i++)
        {
            string p = shown[i];
            string name = (Path.GetFileName(p.TrimEnd('\\', '/')) is { Length: > 0 } n ? n : p).Replace("&", "&&");
            string text = (i < 9 ? $"&{i + 1}  " : "     ") + name;
            var item = new ToolStripMenuItem(text) { ToolTipText = p };
            item.Click += (_, _) =>
            {
                if (Directory.Exists(p)) OpenProject(p); else Warn($"That folder no longer exists:\n{p}");
            };
            menu.DropDownItems.Add(item);
        }
        menu.DropDownItems.Add(new ToolStripSeparator());
        menu.DropDownItems.Add(new ToolStripMenuItem("Clear list", null, (_, _) =>
        {
            _s.RecentProjects.Clear();
            if (_root is not null) _s.AddRecent(_root);
        }));
    }

    ToolStrip BuildProjectStrip()
    {
        var ts = new ToolStrip { GripStyle = ToolStripGripStyle.Hidden, Padding = new Padding(6, 2, 6, 2), BackColor = SystemColors.ControlLight };
        ts.Items.Add(new ToolStripLabel("Target:") { Font = new Font(Font, FontStyle.Bold) });
        _targetLabel.Text = "(none — select a .cto file)";
        ts.Items.Add(_targetLabel);
        ts.Items.Add(Btn("Use certo.toml entry", "Target the entry file from certo.toml", (_, _) =>
        {
            if (_info?.EntryFullPath is { } e) SetTarget(e); else Warn("No [build] entry found in certo.toml.");
        }));
        return ts;
    }

    ToolStrip BuildCommandStrip()
    {
        var ts = new ToolStrip { GripStyle = ToolStripGripStyle.Hidden, Padding = new Padding(4, 2, 4, 2) };

        ToolStripButton Cmd(string text, string tip, Func<Task> run, bool bold = false)
        {
            var b = Btn(text, tip, async (_, _) => await run());
            if (bold) b.Font = new Font(Font, FontStyle.Bold);
            _cmdItems.Add(b);
            return b;
        }
        ToolStripMenuItem Item(string text, Func<Task> run)
        {
            var m = new ToolStripMenuItem(text);
            m.Click += async (_, _) => await run();
            return m;
        }
        ToolStripDropDownButton Drop(string text, params ToolStripItem[] items)
        {
            var d = new ToolStripDropDownButton(text) { DisplayStyle = ToolStripItemDisplayStyle.Text };
            d.DropDownItems.AddRange(items);
            _cmdItems.Add(d);
            return d;
        }

        ts.Items.Add(Cmd("Check", "Type-check only (F6)", () => RunCheck()));
        ts.Items.Add(Cmd("Build", "Compile to a native executable (Ctrl+B)", () => RunBuild(), bold: true));
        ts.Items.Add(Cmd("Run", "Compile and run (F5)", () => RunProgram()));
        ts.Items.Add(Cmd("Test", "Run test blocks (F8)", () => RunSimple("test", "test")));
        ts.Items.Add(new ToolStripSeparator());
        ts.Items.Add(Drop("Format",
            Item("Check formatting (fmt --check)", () => RunFmt(check: true)),
            Item("Format files in place", () => RunFmt(check: false))));
        ts.Items.Add(Cmd("Lint", "Static analysis", () => RunSimple("lint", "lint")));
        ts.Items.Add(Drop("More",
            Item("Bench", () => RunTargetOnly("bench", new List<string> { "bench" })),
            Item("Generate docs → <out>/docs", () => RunDocs()),
            Item("Emit C source (--emit-c)", () => RunTargetOnly("emit-c", new List<string> { "build" }, "--emit-c")),
            Item("Build DLL (--emit-dll)", () => RunDll()),
            Item("Audit dependencies", () => RunProjectCmd("audit", "audit")),
            new ToolStripSeparator(),
            Item("Locate certo.exe…", () => { LocateCerto(); return Task.CompletedTask; }),
            Item("Locate VS Code…", () => { LocateCode(); return Task.CompletedTask; }),
            Item("Set output folder…", () => { SetOutputDir(); return Task.CompletedTask; })));
        ts.Items.Add(Drop("Database",
            Item("Status", () => RunProjectCmd("db status", "db", "status")),
            Item("Migrate", () => RunProjectCmd("db migrate", "db", "migrate")),
            Item("Migrate (dry run)", () => RunProjectCmd("db migrate --dry-run", "db", "migrate", "--dry-run")),
            Item("Rollback last migration…", () => RunRollback()),
            Item("New migration…", () => RunNewMigration())));
        ts.Items.Add(Cmd("Pipeline", "fmt --check → check → lint → test → build", () => RunPipeline()));
        _stopBtn = Btn("Stop", "Stop the running command (Shift+F5)", (_, _) => _cts?.Cancel());
        _stopBtn.Enabled = false;
        ts.Items.Add(_stopBtn);

        // Rarely-changed toggles live in one menu; Scope and Args stay visible at the right edge.
        _releaseBtn = new ToolStripMenuItem("Release build") { CheckOnClick = true, Checked = _s.Release, ToolTipText = "Pass --release (uses [targets.production])" };
        _verboseBtn = new ToolStripMenuItem("Verbose (-v)") { CheckOnClick = true, Checked = _s.Verbose };
        _keepBtn = new ToolStripMenuItem("Pipeline: keep going after failure") { CheckOnClick = true, Checked = _s.KeepGoing };
        var options = new ToolStripDropDownButton("Options") { DisplayStyle = ToolStripItemDisplayStyle.Text, Alignment = ToolStripItemAlignment.Right };
        options.DropDownItems.AddRange(new ToolStripItem[] { _releaseBtn, _verboseBtn, _keepBtn });

        _args = new ToolStripTextBox { Width = 120, Text = _s.ProgramArgs, Alignment = ToolStripItemAlignment.Right, ToolTipText = "Arguments passed to the program after -- when running" };
        var argsLabel = new ToolStripLabel("Args") { Alignment = ToolStripItemAlignment.Right };
        _scope = new ToolStripComboBox { DropDownStyle = ComboBoxStyle.DropDownList, Width = 100, Alignment = ToolStripItemAlignment.Right, ToolTipText = "Which files fmt / lint / test / check / build act on" };
        _scope.Items.AddRange(new object[] { "Target file", "All files" });
        _scope.SelectedIndex = _s.AllFiles ? 1 : 0;
        var scopeLabel = new ToolStripLabel("Scope") { Alignment = ToolStripItemAlignment.Right };
        // Right-aligned items are laid out right-to-left, so add them in reverse visual order.
        ts.Items.AddRange(new ToolStripItem[] { options, _args, argsLabel, _scope, scopeLabel });
        return ts;
    }

    ContextMenuStrip BuildTreeMenu()
    {
        var m = new ContextMenuStrip();
        NodeInfo? Sel() => _tree.SelectedNode?.Tag as NodeInfo;
        void Add(string text, Action<NodeInfo> act) =>
            m.Items.Add(text, null, (_, _) => { if (Sel() is { Path: not null } n) act(n); });

        Add("Open in VS Code", n => { if (n.IsDir) OpenFolderInCode(n.Path); else OpenInCode(n.Path!); });
        Add("Set as target", n => { if (!n.IsDir) SetTarget(n.Path!); });
        m.Items.Add(new ToolStripSeparator());
        Add("Check this file", async n => { if (!n.IsDir) { SetTarget(n.Path!); await RunCheck(forceTarget: true); } });
        Add("Test this file", async n => { if (!n.IsDir) { SetTarget(n.Path!); await RunSimple("test", "test", forceTarget: true); } });
        Add("Run this file", async n => { if (!n.IsDir) { SetTarget(n.Path!); await RunProgram(); } });
        m.Items.Add(new ToolStripSeparator());
        Add("Reveal in Explorer", n => Process.Start("explorer.exe", n.IsDir ? $"\"{n.Path}\"" : $"/select,\"{n.Path}\""));
        Add("Copy path", n => Clipboard.SetText(n.Path!));
        return m;
    }

    ToolStripButton Btn(string text, string tip, EventHandler onClick) =>
        new(text, null, onClick) { ToolTipText = tip, DisplayStyle = ToolStripItemDisplayStyle.Text };

    protected override void OnKeyDown(KeyEventArgs e)
    {
        base.OnKeyDown(e);
        if (_busy && e.KeyData == (Keys.F5 | Keys.Shift)) { _cts?.Cancel(); e.Handled = true; return; }
        if (e.KeyData == (Keys.O | Keys.Control)) { PickProject(); e.Handled = true; }
        else if (e.KeyData == Keys.F5) { _ = RunProgram(); e.Handled = true; }
        else if (e.KeyData == (Keys.B | Keys.Control)) { _ = RunBuild(); e.Handled = true; }
        else if (e.KeyData == Keys.F6) { _ = RunCheck(); e.Handled = true; }
        else if (e.KeyData == Keys.F8) { _ = RunSimple("test", "test"); e.Handled = true; }
    }

    // ── project handling ─────────────────────────────────────────────────
    void PickProject()
    {
        using var dlg = new FolderBrowserDialog { Description = "Choose a Certo project folder", UseDescriptionForTitle = true, InitialDirectory = _root ?? "" };
        if (dlg.ShowDialog(this) == DialogResult.OK) OpenProject(dlg.SelectedPath);
    }

    void OpenProject(string path)
    {
        _root = Path.GetFullPath(path);
        _info = ProjectInfo.Load(_root);
        _s.ProjectRoot = _root;
        _s.AddRecent(_root);
        Text = $"{_info.Name} — not-ide";
        _target = null;
        if (!string.IsNullOrEmpty(_s.Target) && File.Exists(_s.Target) && _s.Target.StartsWith(_root, StringComparison.OrdinalIgnoreCase))
            SetTarget(_s.Target);
        else if (_info.EntryFullPath is { } e && File.Exists(e)) SetTarget(e);
        else SetTarget(null);
        StartWatcher();
        RefreshTree();
        _diag.Items.Clear(); _errors = _warnings = 0; UpdateDiagHeader();
        Append($"Project: {_root}\n", Color.FromArgb(120, 190, 255));
        if (_info.ManifestPath is null) Append("No certo.toml found — commands will use the selected .cto file.\n", Color.Gray);
        UpdateCertoStatus();
    }

    void StartWatcher()
    {
        _watcher?.Dispose();
        if (_root is null) return;
        try
        {
            _watcher = new FileSystemWatcher(_root) { IncludeSubdirectories = true, EnableRaisingEvents = true,
                NotifyFilter = NotifyFilters.FileName | NotifyFilters.DirectoryName };
            FileSystemEventHandler h = (_, e) => OnFsChange(e.FullPath);
            _watcher.Created += h; _watcher.Deleted += h;
            _watcher.Renamed += (_, e) => OnFsChange(e.FullPath);
        }
        catch { /* watcher is a convenience only */ }
    }

    void OnFsChange(string path)
    {
        if (_root is null || IsIgnoredPath(path)) return;
        BeginInvoke(() => { _refreshDebounce.Stop(); _refreshDebounce.Start(); });
    }

    bool IsIgnoredPath(string full)
    {
        if (_root is null) return true;
        var rel = Path.GetRelativePath(_root, full);
        foreach (var seg in rel.Split(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar))
            if (IgnoredDirs.Contains(seg)) return true;
        return false;
    }

    void SetTarget(string? file)
    {
        _target = file;
        _targetLabel.Text = file is null ? "(none — select a .cto file)"
            : (_root is not null ? Path.GetRelativePath(_root, file) : file);
        if (file is not null && _info?.EntryFullPath is { } e && SamePath(e, file)) _targetLabel.Text += "  (entry)";
    }

    static bool SamePath(string a, string b) =>
        string.Equals(Path.GetFullPath(a), Path.GetFullPath(b), StringComparison.OrdinalIgnoreCase);

    // ── tree building ────────────────────────────────────────────────────
    void RefreshTree()
    {
        if (_root is null || _info is null) return;
        var expanded = new HashSet<string>();
        CollectExpanded(_tree.Nodes, expanded);
        string? selKey = (_tree.SelectedNode?.Tag as NodeInfo)?.Key;
        string filter = _filter.Text.Trim();

        _tree.BeginUpdate();
        _tree.Nodes.Clear();
        var rootNode = new TreeNode(_info.Name) { Tag = new NodeInfo("d:" + _root, _root, true), ToolTipText = _root };
        _tree.Nodes.Add(rootNode);

        switch (_view.SelectedItem?.ToString())
        {
            case "By module": BuildByModule(rootNode, filter); break;
            case "By type": BuildByType(rootNode, filter); break;
            default: BuildFolder(rootNode, _root, filter, isRoot: true); break;
        }

        rootNode.Expand();
        bool expandAll = filter.Length > 0;
        RestoreExpanded(_tree.Nodes, expanded, expandAll, selKey);
        _tree.EndUpdate();
    }

    static void CollectExpanded(TreeNodeCollection nodes, HashSet<string> into)
    {
        foreach (TreeNode n in nodes)
        {
            if (n.IsExpanded && n.Tag is NodeInfo ni) into.Add(ni.Key);
            CollectExpanded(n.Nodes, into);
        }
    }

    void RestoreExpanded(TreeNodeCollection nodes, HashSet<string> expanded, bool all, string? selKey)
    {
        foreach (TreeNode n in nodes)
        {
            if (n.Tag is NodeInfo ni)
            {
                if (all || expanded.Contains(ni.Key)) n.Expand();
                if (selKey != null && ni.Key == selKey) _tree.SelectedNode = n;
            }
            RestoreExpanded(n.Nodes, expanded, all, selKey);
        }
    }

    bool BuildFolder(TreeNode parent, string dir, string filter, bool isRoot = false)
    {
        TreeNode node = isRoot ? parent : new TreeNode(Path.GetFileName(dir)) { Tag = new NodeInfo("d:" + dir, dir, true) };
        foreach (var sub in SafeDirs(dir))
        {
            if (IgnoredDirs.Contains(Path.GetFileName(sub))) continue;
            BuildFolder(node, sub, filter);
        }
        foreach (var f in SafeFiles(dir))
            if (ShowFile(f) && Matches(f, filter)) node.Nodes.Add(FileNode(f));

        if (isRoot) return true;
        if (node.Nodes.Count == 0 && filter.Length > 0) return false;
        parent.Nodes.Add(node);
        return true;
    }

    void BuildByModule(TreeNode root, string filter)
    {
        var groups = new SortedDictionary<string, List<string>>(StringComparer.OrdinalIgnoreCase);
        foreach (var f in AllCtoFiles())
        {
            if (!Matches(f, filter)) continue;
            var mod = Scanner.Meta(f).Module ?? "(no module)";
            if (!groups.TryGetValue(mod, out var list)) groups[mod] = list = new();
            list.Add(f);
        }
        foreach (var (mod, files) in groups)
        {
            var g = new TreeNode($"{mod}  ({files.Count})") { Tag = new NodeInfo("g:" + mod, null, true) };
            foreach (var f in files.OrderBy(x => x, StringComparer.OrdinalIgnoreCase)) g.Nodes.Add(FileNode(f, showDir: true));
            root.Nodes.Add(g);
        }
    }

    void BuildByType(TreeNode root, string filter)
    {
        var groups = new SortedDictionary<string, List<string>>(StringComparer.OrdinalIgnoreCase);
        foreach (var f in AllFiles())
        {
            if (!Matches(f, filter)) continue;
            string g = GroupOf(Path.GetExtension(f));
            if (!groups.TryGetValue(g, out var list)) groups[g] = list = new();
            list.Add(f);
        }
        foreach (var (name, files) in groups)
        {
            var g = new TreeNode($"{name}  ({files.Count})") { Tag = new NodeInfo("g:" + name, null, true) };
            foreach (var f in files.OrderBy(x => x, StringComparer.OrdinalIgnoreCase)) g.Nodes.Add(FileNode(f, showDir: true));
            root.Nodes.Add(g);
        }
    }

    static string GroupOf(string ext) => ext.ToLowerInvariant() switch
    {
        ".cto" => "1 Certo sources",
        ".toml" or ".json" or ".yaml" or ".yml" or ".schema" => "2 Config & schemas",
        ".md" or ".txt" => "3 Docs",
        ".ps1" or ".cmd" or ".bat" or ".ion" or ".sh" or ".py" => "4 Scripts",
        ".c" or ".h" or ".rs" or ".cs" => "5 C / Rust / other code",
        _ => "6 Other",
    };

    TreeNode FileNode(string path, bool showDir = false)
    {
        string text = Path.GetFileName(path);
        if (showDir && _root is not null)
        {
            var dir = Path.GetDirectoryName(Path.GetRelativePath(_root, path));
            if (!string.IsNullOrEmpty(dir)) text += $"   — {dir}";
        }
        var color = SystemColors.WindowText;
        string ext = Path.GetExtension(path).ToLowerInvariant();
        var tips = new List<string> { path };
        if (ext == ".cto")
        {
            var m = Scanner.Meta(path);
            if (m.HasMain) text += "  ▶";
            if (m.HasTests) text += "  ✓tests";
            if (_info?.EntryFullPath is { } e && SamePath(e, path)) { text += "  ◀ entry"; color = Color.FromArgb(0, 102, 204); }
            if (m.Module != null) tips.Add($"module {m.Module}");
            if (m.HasMain) tips.Add("has fn main");
            if (m.HasTests) tips.Add("has test blocks");
        }
        else if (ext is ".md" or ".txt") color = Color.DimGray;
        else if (ext is ".toml" or ".json") color = Color.FromArgb(160, 90, 0);
        return new TreeNode(text) { Tag = new NodeInfo("f:" + path, path, false), ForeColor = color, ToolTipText = string.Join("\n", tips) };
    }

    static bool Matches(string path, string filter) =>
        filter.Length == 0 || path.Contains(filter, StringComparison.OrdinalIgnoreCase);

    static bool ShowFile(string f) => !HiddenExts.Contains(Path.GetExtension(f));

    static IEnumerable<string> SafeDirs(string d)
    {
        try { return Directory.GetDirectories(d).OrderBy(x => x, StringComparer.OrdinalIgnoreCase).ToList(); }
        catch { return Array.Empty<string>(); }
    }
    static IEnumerable<string> SafeFiles(string d)
    {
        try { return Directory.GetFiles(d).OrderBy(x => x, StringComparer.OrdinalIgnoreCase).ToList(); }
        catch { return Array.Empty<string>(); }
    }

    IEnumerable<string> AllFiles()
    {
        if (_root is null) yield break;
        var stack = new Stack<string>(); stack.Push(_root);
        while (stack.Count > 0)
        {
            var d = stack.Pop();
            foreach (var sub in SafeDirs(d)) if (!IgnoredDirs.Contains(Path.GetFileName(sub))) stack.Push(sub);
            foreach (var f in SafeFiles(d)) if (ShowFile(f)) yield return f;
        }
    }

    List<string> AllCtoFiles() =>
        AllFiles().Where(f => f.EndsWith(".cto", StringComparison.OrdinalIgnoreCase))
                  .OrderBy(f => f, StringComparer.OrdinalIgnoreCase).ToList();

    // ── command construction ─────────────────────────────────────────────
    bool AllScope => _scope.SelectedIndex == 1;

    IEnumerable<string> BuildFlags()
    {
        if (_releaseBtn.Checked) yield return "--release";
        if (_verboseBtn.Checked) yield return "-v";
    }

    string? TargetOrWarn()
    {
        var t = _target ?? _info?.EntryFullPath;
        if (t is null || !File.Exists(t)) { Warn("Select a .cto file in the tree first (or use the certo.toml entry)."); return null; }
        return t;
    }

    string OutPath(string file, string ext = ".exe")
    {
        string dir = Path.Combine(_root!, _s.OutputDir);
        Directory.CreateDirectory(dir);
        return Path.Combine(dir, Path.GetFileNameWithoutExtension(file) + ext);
    }

    string Rel(string p) => _root is null ? p : Path.GetRelativePath(_root, p);

    Task RunCheck(bool forceTarget = false)
    {
        if (_root is null) return NoProject();
        var files = (AllScope && !forceTarget) ? AllCtoFiles() : TargetOrWarn() is { } t ? new List<string> { t } : null;
        if (files is null || files.Count == 0) return Task.CompletedTask;
        var steps = files.Select(f => new Step($"check {Rel(f)}",
            new List<string> { "check", f }.Concat(_verboseBtn.Checked ? new[] { "-v" } : Array.Empty<string>()).ToList())).ToList();
        return RunSteps("Check", steps);
    }

    Task RunBuild()
    {
        if (_root is null) return NoProject();
        var steps = new List<Step>();
        if (AllScope)
        {
            foreach (var f in AllCtoFiles().Where(x => Scanner.Meta(x).HasMain))
                steps.Add(BuildStep(f));
            if (steps.Count == 0) { Warn("No .cto files with fn main found."); return Task.CompletedTask; }
        }
        else
        {
            var t = TargetOrWarn(); if (t is null) return Task.CompletedTask;
            steps.Add(BuildStep(t));
        }
        return RunSteps("Build", steps);
    }

    Step BuildStep(string file)
    {
        // The manifest entry builds with plain `certo build` so certo.toml's output/profile rules apply.
        if (_info?.ManifestPath != null && _info.EntryFullPath is { } e && SamePath(e, file))
            return new Step("build (certo.toml)", new List<string> { "build" }.Concat(BuildFlags()).ToList());
        return new Step($"build {Rel(file)}",
            new List<string> { "build", file, "-o", OutPath(file) }.Concat(BuildFlags()).ToList());
    }

    Task RunProgram()
    {
        if (_root is null) return NoProject();
        var t = TargetOrWarn(); if (t is null) return Task.CompletedTask;
        var args = new List<string> { "run", t };
        args.AddRange(BuildFlags());
        var prog = ArgSplitter.Split(_args.Text);
        if (prog.Count > 0) { args.Add("--"); args.AddRange(prog); }
        return RunSteps("Run", new List<Step> { new($"run {Rel(t)}", args) });
    }

    /// <summary>fmt/lint/test style commands that accept many files.</summary>
    Task RunSimple(string title, string verb, bool forceTarget = false)
    {
        if (_root is null) return NoProject();
        List<string> files;
        if (AllScope && !forceTarget)
        {
            files = AllCtoFiles();
            if (verb == "test") files = files.Where(f => Scanner.Meta(f).HasTests).ToList();
            if (files.Count == 0) { Warn(verb == "test" ? "No .cto files with test blocks found." : "No .cto files found."); return Task.CompletedTask; }
        }
        else
        {
            var t = TargetOrWarn(); if (t is null) return Task.CompletedTask;
            files = new List<string> { t };
        }
        var args = new List<string> { verb };
        if (verb == "test") args.Add("--no-color");
        args.AddRange(files);
        return RunSteps(title, new List<Step> { new($"{verb} ({files.Count} file{(files.Count == 1 ? "" : "s")})", args) });
    }

    Task RunFmt(bool check)
    {
        if (_root is null) return NoProject();
        List<string> files;
        if (AllScope) files = AllCtoFiles();
        else { var t = TargetOrWarn(); if (t is null) return Task.CompletedTask; files = new List<string> { t }; }
        if (files.Count == 0) return Task.CompletedTask;
        if (!check && files.Count > 1 &&
            MessageBox.Show(this, $"Format {files.Count} files in place?", "Format", MessageBoxButtons.OKCancel, MessageBoxIcon.Question) != DialogResult.OK)
            return Task.CompletedTask;
        var args = new List<string> { "fmt" };
        if (check) args.Add("--check");
        args.AddRange(files);
        return RunSteps(check ? "Format check" : "Format", new List<Step> { new(check ? "fmt --check" : "fmt", args) });
    }

    Task RunTargetOnly(string title, List<string> verb, params string[] extra)
    {
        if (_root is null) return NoProject();
        var t = TargetOrWarn(); if (t is null) return Task.CompletedTask;
        var args = new List<string>(verb) { t };
        args.AddRange(extra);
        return RunSteps(title, new List<Step> { new($"{title} {Rel(t)}", args) });
    }

    Task RunDocs()
    {
        if (_root is null) return NoProject();
        var t = TargetOrWarn(); if (t is null) return Task.CompletedTask;
        string dir = Path.Combine(_root, _s.OutputDir, "docs");
        Directory.CreateDirectory(dir);
        return RunSteps("Docs", new List<Step> { new($"doc {Rel(t)}", new List<string> { "doc", t, "-o", dir }) });
    }

    Task RunDll()
    {
        if (_root is null) return NoProject();
        var t = TargetOrWarn(); if (t is null) return Task.CompletedTask;
        return RunSteps("Build DLL", new List<Step> { new($"build --emit-dll {Rel(t)}",
            new List<string> { "build", t, "--emit-dll", "-o", OutPath(t, ".dll") }) });
    }

    Task RunProjectCmd(string title, params string[] args)
    {
        if (_root is null) return NoProject();
        return RunSteps(title, new List<Step> { new($"certo {string.Join(' ', args)}", args.ToList()) });
    }

    Task RunRollback()
    {
        if (MessageBox.Show(this, "Roll back the last migration?", "Database", MessageBoxButtons.OKCancel, MessageBoxIcon.Warning) != DialogResult.OK)
            return Task.CompletedTask;
        return RunProjectCmd("db rollback", "db", "rollback");
    }

    Task RunNewMigration()
    {
        var name = Prompt.Ask(this, "New migration", "Migration name:");
        return string.IsNullOrWhiteSpace(name) ? Task.CompletedTask : RunProjectCmd("db create", "db", "create", name.Trim());
    }

    Task RunPipeline()
    {
        if (_root is null) return NoProject();
        var files = AllScope ? AllCtoFiles() : TargetOrWarn() is { } t ? new List<string> { t } : null;
        if (files is null || files.Count == 0) return Task.CompletedTask;
        var steps = new List<Step>
        {
            new("fmt --check", new List<string> { "fmt", "--check" }.Concat(files).ToList()),
        };
        steps.AddRange(files.Select(f => new Step($"check {Rel(f)}", new List<string> { "check", f })));
        steps.Add(new("lint", new List<string> { "lint" }.Concat(files).ToList()));
        var testFiles = files.Where(f => Scanner.Meta(f).HasTests).ToList();
        if (testFiles.Count > 0)
            steps.Add(new("test", new List<string> { "test", "--no-color" }.Concat(testFiles).ToList()));
        foreach (var f in files.Where(x => Scanner.Meta(x).HasMain || !AllScope)) steps.Add(BuildStep(f));
        return RunSteps("Pipeline", steps);
    }

    Task NoProject() { Warn("Open a project folder first."); return Task.CompletedTask; }

    // ── running ──────────────────────────────────────────────────────────
    async Task RunSteps(string title, List<Step> steps)
    {
        if (_busy || _root is null) return;
        var certo = Locator.FindCerto(_s.CertoPath, _root);
        if (certo is null)
        {
            if (MessageBox.Show(this, "Could not find certo.exe. Locate it now?", "certo not found", MessageBoxButtons.YesNo, MessageBoxIcon.Question) == DialogResult.Yes)
                LocateCerto();
            certo = Locator.FindCerto(_s.CertoPath, _root);
            if (certo is null) return;
        }

        _cts = new CancellationTokenSource();
        var ct = _cts.Token;
        SetBusy(true, title);
        _diag.Items.Clear(); _errors = _warnings = 0; UpdateDiagHeader();
        _log.Clear();
        Append($"▶ {title}   ({certo})\n", Color.FromArgb(120, 190, 255));

        bool keepGoing = _keepBtn.Checked;
        int ran = 0, failed = 0, warned = 0;
        foreach (var step in steps)
        {
            if (ct.IsCancellationRequested) break;
            ran++;
            Append($"\n==> certo {string.Join(' ', step.Args.Select(Quote))}\n", Color.FromArgb(86, 156, 214));
            var parser = new DiagnosticParser();
            int errorsBefore = _errors, warningsBefore = _warnings;
            int code;
            try
            {
                code = await _runner.RunAsync(certo, step.Args, _root,
                    line => BeginInvoke(() => OnOutputLine(line, parser)), ct);
            }
            catch (Exception ex)
            {
                Append($"Could not start certo: {ex.Message}\n", Color.IndianRed);
                code = -2;
            }
            while (parser.Flush() is { } rest) AddDiagnostic(rest);

            if (ct.IsCancellationRequested) { Append("Stopped.\n", Color.Orange); break; }
            // lint and fmt --check exit 1 when they only found warnings; that isn't a failure.
            bool warnOnly = code == 1 && _errors == errorsBefore && _warnings > warningsBefore;
            if (code == 0) Append($"✔ {step.Title} — ok\n", Color.FromArgb(106, 190, 106));
            else if (warnOnly)
            {
                warned++;
                Append($"⚠ {step.Title} — {_warnings - warningsBefore} warning(s)\n", Color.FromArgb(230, 190, 90));
            }
            else
            {
                failed++;
                Append($"✘ {step.Title} — exit code {code}\n", Color.IndianRed);
                if (title.StartsWith("db") && code != 0)
                    Append("  (database commands need DATABASE_URL — set it in the environment or a .env file)\n", Color.Gray);
                if (!keepGoing && steps.Count > 1) { Append("Stopping at first failure (enable \"Keep going\" to continue).\n", Color.Gray); break; }
            }
        }

        _clock.Stop();
        bool cancelled = ct.IsCancellationRequested;
        string summary = cancelled ? "Stopped"
            : failed > 0 ? $"Failed ({failed} of {ran} step{(ran == 1 ? "" : "s")})"
            : warned > 0 ? "Finished with warnings" : "Succeeded";
        Append($"\n{summary} — {_errors} error(s), {_warnings} warning(s) in {FormatElapsed(_clock.Elapsed)}\n",
            cancelled || failed > 0 ? Color.IndianRed : warned > 0 ? Color.FromArgb(230, 190, 90) : Color.FromArgb(106, 190, 106));
        SetBusy(false, $"{title}: {summary}");
        _cts.Dispose(); _cts = null;

        _history.Add(new RunRecord(
            DateTime.Now, _info?.Name ?? Path.GetFileName(_root.TrimEnd('\\', '/')), title,
            AllScope ? "All files" : _target is null ? "—" : Path.GetFileName(_target),
            cancelled ? RunResult.Stopped : failed > 0 ? RunResult.Failed : warned > 0 ? RunResult.Warnings : RunResult.Succeeded,
            _errors, _warnings, _clock.Elapsed.TotalSeconds));
    }

    // ── run history window ───────────────────────────────────────────────
    void ShowHistory()
    {
        using var dlg = new Form
        {
            Text = "Run history — not-ide", StartPosition = FormStartPosition.CenterParent, Font = Font,
            Size = new Size(1040, 560), MinimumSize = new Size(760, 320),
        };
        var list = new ListView { Dock = DockStyle.Fill, View = View.Details, FullRowSelect = true, HideSelection = false, GridLines = false };
        list.Columns.Add("When", 210); list.Columns.Add("Project", 120); list.Columns.Add("Command", 120);
        list.Columns.Add("Target", 190); list.Columns.Add("Result", 110); list.Columns.Add("Errors", 70);
        list.Columns.Add("Warnings", 90); list.Columns.Add("Time", 80);

        void Fill()
        {
            list.Items.Clear();
            foreach (var r in _history.Items)
            {
                var item = new ListViewItem(new[]
                {
                    r.When.ToString("ddd d MMM  HH:mm:ss"), r.Project,
                    r.Command.Length > 0 ? char.ToUpper(r.Command[0]) + r.Command[1..] : r.Command, r.Target, r.Result.ToString(),
                    r.Errors.ToString(), r.Warnings.ToString(), FormatElapsed(TimeSpan.FromSeconds(r.Seconds)),
                });
                item.ForeColor = r.Result switch
                {
                    RunResult.Succeeded => Color.SeaGreen, RunResult.Warnings => Color.DarkGoldenrod,
                    RunResult.Failed => Color.Firebrick, _ => Color.DimGray,
                };
                list.Items.Add(item);
            }
        }
        Fill();

        var bar = new FlowLayoutPanel { Dock = DockStyle.Bottom, FlowDirection = FlowDirection.RightToLeft, AutoSize = true, Padding = new Padding(8) };
        var close = new Button { Text = "Close", AutoSize = true, DialogResult = DialogResult.OK };
        var clear = new Button { Text = "Clear history", AutoSize = true };
        clear.Click += (_, _) =>
        {
            if (_history.Items.Count == 0) return;
            if (MessageBox.Show(dlg, $"Delete all {_history.Items.Count} recorded runs?", "Clear history",
                    MessageBoxButtons.YesNo, MessageBoxIcon.Question) == DialogResult.Yes)
            { _history.Clear(); Fill(); }
        };
        var count = new Label { AutoSize = true, Padding = new Padding(0, 6, 12, 0), ForeColor = SystemColors.GrayText, Text = $"Keeps the last {RunHistory.Max} runs" };
        bar.Controls.AddRange(new Control[] { close, clear, count });
        dlg.Controls.Add(list);
        dlg.Controls.Add(bar);
        dlg.AcceptButton = close;
        dlg.CancelButton = close;
        dlg.ShowDialog(this);
    }

    static string Quote(string a) => a.Contains(' ') ? $"\"{a}\"" : a;

    void OnOutputLine(string raw, DiagnosticParser parser)
    {
        string line = Ansi.Strip(raw);
        Color c = Color.FromArgb(212, 212, 212);
        string t = line.TrimStart();
        if (t.StartsWith("error", StringComparison.OrdinalIgnoreCase) || line.Contains("FAILED") || line.Contains("panicked")) c = Color.FromArgb(244, 112, 112);
        else if (t.StartsWith("warning", StringComparison.OrdinalIgnoreCase)) c = Color.FromArgb(230, 190, 90);
        else if (t.StartsWith("-->")) c = Color.FromArgb(120, 200, 230);
        else if (line.TrimEnd().EndsWith(" ok") || t.StartsWith("ok")) c = Color.FromArgb(106, 190, 106);
        Append(line + "\n", c);
        if (parser.Feed(line) is { } d) AddDiagnostic(d);
    }

    void AddDiagnostic(Diagnostic d)
    {
        if (d.Severity == Sev.Error) _errors++; else if (d.Severity == Sev.Warning) _warnings++;
        string loc = d.File is null ? "" : $"{Rel(ResolvePath(d.File) ?? d.File)}:{d.Line}:{d.Col}";
        var item = new ListViewItem(new[] { d.Severity.ToString(), d.Code, d.Message, loc }) { Tag = d };
        item.ForeColor = d.Severity switch { Sev.Error => Color.Firebrick, Sev.Warning => Color.DarkGoldenrod, _ => Color.DimGray };
        _diag.Items.Add(item);
        UpdateDiagHeader();
    }

    void UpdateDiagHeader() =>
        _diagHeader.Text = _errors + _warnings == 0 ? "Problems" : $"Problems — {_errors} error(s), {_warnings} warning(s)";

    void SetBusy(bool busy, string state)
    {
        _busy = busy;
        foreach (var i in _cmdItems) i.Enabled = !busy;
        _stopBtn.Enabled = busy;
        _stState.Text = busy ? $"Running {state}…" : state;
        if (busy) { _clock.Restart(); _tick.Start(); }
        else { _tick.Stop(); _stTime.Text = FormatElapsed(_clock.Elapsed); }
    }

    static string FormatElapsed(TimeSpan t) => t.TotalSeconds < 60 ? $"{t.TotalSeconds:0.0}s" : $"{(int)t.TotalMinutes}m {t.Seconds:00}s";

    // ── output / navigation helpers ──────────────────────────────────────
    void Append(string text, Color color)
    {
        _log.SelectionStart = _log.TextLength;
        _log.SelectionLength = 0;
        _log.SelectionColor = color;
        _log.AppendText(text);
        _log.SelectionColor = _log.ForeColor;
        _log.ScrollToCaret();
    }

    string? ResolvePath(string file)
    {
        if (Path.IsPathRooted(file)) return File.Exists(file) ? file : null;
        var candidates = new List<string>();
        if (_root != null) candidates.Add(Path.Combine(_root, file));
        if (_target != null) candidates.Add(Path.Combine(Path.GetDirectoryName(_target)!, file));
        return candidates.FirstOrDefault(File.Exists);
    }

    void OpenLocation(string file, int line, int col)
    {
        var p = ResolvePath(file);
        if (p is null) { Warn($"Can't find {file} on disk."); return; }
        OpenInCode(p, line, col);
    }

    void OpenLogLine()
    {
        int idx = _log.GetCharIndexFromPosition(_log.PointToClient(Cursor.Position));
        int ln = _log.GetLineFromCharIndex(idx);
        if (ln < 0 || ln >= _log.Lines.Length) return;
        if (DiagnosticParser.TryLocation(_log.Lines[ln], out var f, out var l, out var c)) OpenLocation(f, l, c);
    }

    void OpenInCode(string path, int line = 0, int col = 0)
    {
        var code = Locator.FindCode(_s.CodePath);
        if (code is null) { Warn("VS Code wasn't found. Use More ▸ Locate VS Code…"); return; }
        var target = line > 0 ? $"{path}:{line}:{Math.Max(col, 1)}" : path;
        var args = new List<string>();
        if (_root != null) args.Add(_root);
        args.Add("-g"); args.Add(target);
        LaunchCode(code, args);
    }

    void OpenFolderInCode(string? folder = null)
    {
        var dir = folder ?? _root;
        if (dir is null) { Warn("Open a project folder first."); return; }
        var code = Locator.FindCode(_s.CodePath);
        if (code is null) { Warn("VS Code wasn't found. Use More ▸ Locate VS Code…"); return; }
        LaunchCode(code, new List<string> { dir });
    }

    void LaunchCode(string code, List<string> args)
    {
        try
        {
            var psi = new ProcessStartInfo { UseShellExecute = false, CreateNoWindow = true };
            if (code.EndsWith(".cmd", StringComparison.OrdinalIgnoreCase))
            {
                psi.FileName = "cmd.exe";
                psi.Arguments = "/c \"\"" + code + "\" " + string.Join(' ', args.Select(a => "\"" + a + "\"")) + "\"";
            }
            else
            {
                psi.FileName = code;
                foreach (var a in args) psi.ArgumentList.Add(a);
            }
            Process.Start(psi);
        }
        catch (Exception ex) { Warn("Could not start VS Code: " + ex.Message); }
    }

    void Warn(string msg) => MessageBox.Show(this, msg, "not-ide", MessageBoxButtons.OK, MessageBoxIcon.Information);

    // ── settings helpers ─────────────────────────────────────────────────
    void LocateCerto()
    {
        using var dlg = new OpenFileDialog { Title = "Locate certo.exe", Filter = "certo.exe|certo.exe|Executables|*.exe" };
        if (dlg.ShowDialog(this) != DialogResult.OK) return;
        _s.CertoPath = dlg.FileName; _s.Save();
        UpdateCertoStatus();
        _ = ShowCertoVersionAsync();
    }

    void LocateCode()
    {
        using var dlg = new OpenFileDialog { Title = "Locate Code.exe or code.cmd", Filter = "VS Code|Code.exe;code.cmd|All files|*.*" };
        if (dlg.ShowDialog(this) == DialogResult.OK) { _s.CodePath = dlg.FileName; _s.Save(); }
    }

    void SetOutputDir()
    {
        var v = Prompt.Ask(this, "Output folder", "Build output folder (relative to the project):", _s.OutputDir);
        if (!string.IsNullOrWhiteSpace(v)) { _s.OutputDir = v.Trim(); _s.Save(); }
    }

    void UpdateCertoStatus()
    {
        var c = Locator.FindCerto(_s.CertoPath, _root);
        _stCerto.Text = c is null ? "certo: not found (More ▸ Locate certo.exe…)" : $"certo: {c}";
    }

    async Task ShowCertoVersionAsync()
    {
        var certo = Locator.FindCerto(_s.CertoPath, _root);
        if (certo is null) { UpdateCertoStatus(); return; }
        string first = "";
        try
        {
            await _runner.RunAsync(certo, new[] { "--version" }, _root ?? Environment.CurrentDirectory,
                l => { if (first.Length == 0) first = l.Trim(); }, CancellationToken.None);
        }
        catch { }
        _stCerto.Text = $"{(first.Length > 0 ? first + "  —  " : "")}{certo}";
    }
}

/// <summary>Tiny single-line input dialog.</summary>
static class Prompt
{
    public static string? Ask(IWin32Window owner, string title, string label, string initial = "")
    {
        using var f = new Form
        {
            Text = title, FormBorderStyle = FormBorderStyle.FixedDialog, StartPosition = FormStartPosition.CenterParent,
            MinimizeBox = false, MaximizeBox = false, ClientSize = new Size(420, 110),
        };
        var l = new Label { Text = label, Left = 12, Top = 12, Width = 396 };
        var t = new TextBox { Left = 12, Top = 36, Width = 396, Text = initial };
        var ok = new Button { Text = "OK", Left = 252, Top = 70, Width = 75, DialogResult = DialogResult.OK };
        var cancel = new Button { Text = "Cancel", Left = 333, Top = 70, Width = 75, DialogResult = DialogResult.Cancel };
        f.Controls.AddRange(new Control[] { l, t, ok, cancel });
        f.AcceptButton = ok; f.CancelButton = cancel;
        return f.ShowDialog(owner) == DialogResult.OK ? t.Text : null;
    }
}
