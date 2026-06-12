const {
  Document, Packer, Paragraph, TextRun, Table, TableRow, TableCell,
  Header, Footer, AlignmentType, HeadingLevel, BorderStyle, WidthType,
  ShadingType, VerticalAlign, PageNumber, PageBreak, LevelFormat,
  TableOfContents, Bookmark, InternalHyperlink, ExternalHyperlink
} = require('docx');
const fs = require('fs');

// ── Colour palette ──────────────────────────────────────────────────────────
const BLUE       = "1F4E79";
const LIGHT_BLUE = "D6E4F0";
const MID_BLUE   = "2E75B6";
const ACCENT     = "C55A11";
const CODE_BG    = "F2F2F2";
const HEADER_FG  = "FFFFFF";

// ── Helpers ─────────────────────────────────────────────────────────────────

function h1(text, bookmarkId) {
  const run = new TextRun({ text, bold: true, size: 36, color: HEADER_FG, font: "Arial" });
  const children = bookmarkId
    ? [new Bookmark({ id: bookmarkId, children: [run] })]
    : [run];
  return new Paragraph({
    heading: HeadingLevel.HEADING_1,
    children,
    shading: { fill: BLUE, type: ShadingType.CLEAR },
    spacing: { before: 360, after: 120 },
  });
}

function h2(text, bookmarkId) {
  const run = new TextRun({ text, bold: true, size: 28, color: MID_BLUE, font: "Arial" });
  const children = bookmarkId
    ? [new Bookmark({ id: bookmarkId, children: [run] })]
    : [run];
  return new Paragraph({
    heading: HeadingLevel.HEADING_2,
    children,
    border: { bottom: { style: BorderStyle.SINGLE, size: 4, color: MID_BLUE } },
    spacing: { before: 280, after: 80 },
  });
}

function h3(text) {
  return new Paragraph({
    heading: HeadingLevel.HEADING_3,
    children: [new TextRun({ text, bold: true, size: 24, color: ACCENT, font: "Arial" })],
    spacing: { before: 200, after: 60 },
  });
}

function para(text, opts = {}) {
  return new Paragraph({
    children: [new TextRun({ text, font: "Arial", size: 22, ...opts })],
    spacing: { before: 60, after: 100 },
  });
}

function richPara(runs) {
  return new Paragraph({
    children: runs,
    spacing: { before: 60, after: 100 },
  });
}

function code(lines) {
  // Monospace code block
  return lines.map((line, i) => new Paragraph({
    children: [new TextRun({ text: line === "" ? " " : line, font: "Courier New", size: 18, color: "1A1A1A" })],
    shading: { fill: CODE_BG, type: ShadingType.CLEAR },
    spacing: { before: i === 0 ? 80 : 0, after: i === lines.length - 1 ? 80 : 0 },
    indent: { left: 360 },
  }));
}

function bullet(text, level = 0) {
  return new Paragraph({
    numbering: { reference: "bullets", level },
    children: [new TextRun({ text, font: "Arial", size: 22 })],
    spacing: { before: 40, after: 40 },
  });
}

function note(text) {
  return new Paragraph({
    children: [
      new TextRun({ text: "Note: ", bold: true, font: "Arial", size: 20, color: ACCENT }),
      new TextRun({ text, font: "Arial", size: 20, italics: true }),
    ],
    shading: { fill: "FFF3E0", type: ShadingType.CLEAR },
    border: { left: { style: BorderStyle.SINGLE, size: 8, color: ACCENT } },
    indent: { left: 300 },
    spacing: { before: 80, after: 80 },
  });
}

function inlineCode(text) {
  return new TextRun({ text, font: "Courier New", size: 20, shading: { fill: CODE_BG, type: ShadingType.CLEAR } });
}

function pageBreak() {
  return new Paragraph({ children: [new PageBreak()] });
}

function spacer() {
  return new Paragraph({ children: [new TextRun("")], spacing: { before: 80, after: 80 } });
}

// Two-column table for side-by-side comparisons
function sideBySide(leftHeader, rightHeader, leftLines, rightLines) {
  const border = { style: BorderStyle.SINGLE, size: 1, color: "CCCCCC" };
  const borders = { top: border, bottom: border, left: border, right: border };
  const cellMargins = { top: 80, bottom: 80, left: 120, right: 120 };
  const half = 4680;

  function codeCell(lines, header, isHeader = false) {
    const paragraphs = [];
    if (header) {
      paragraphs.push(new Paragraph({
        children: [new TextRun({ text: header, bold: true, font: "Arial", size: 20, color: isHeader ? HEADER_FG : "333333" })],
        shading: { fill: isHeader ? MID_BLUE : "E8E8E8", type: ShadingType.CLEAR },
        spacing: { before: 40, after: 40 },
      }));
    }
    lines.forEach(l => paragraphs.push(new Paragraph({
      children: [new TextRun({ text: l === "" ? " " : l, font: "Courier New", size: 18 })],
      shading: { fill: CODE_BG, type: ShadingType.CLEAR },
      spacing: { before: 0, after: 0 },
    })));
    return new TableCell({ borders, margins: cellMargins, width: { size: half, type: WidthType.DXA }, children: paragraphs });
  }

  return new Table({
    width: { size: 9360, type: WidthType.DXA },
    columnWidths: [half, half],
    rows: [new TableRow({ children: [codeCell(leftLines, leftHeader, true), codeCell(rightLines, rightHeader, true)] })],
  });
}

// ── Document ────────────────────────────────────────────────────────────────

const doc = new Document({
  numbering: {
    config: [
      {
        reference: "bullets",
        levels: [
          { level: 0, format: LevelFormat.BULLET, text: "•", alignment: AlignmentType.LEFT,
            style: { paragraph: { indent: { left: 720, hanging: 360 } } } },
          { level: 1, format: LevelFormat.BULLET, text: "◦", alignment: AlignmentType.LEFT,
            style: { paragraph: { indent: { left: 1080, hanging: 360 } } } },
        ]
      }
    ]
  },
  styles: {
    default: { document: { run: { font: "Arial", size: 22 } } },
    paragraphStyles: [
      { id: "Heading1", name: "Heading 1", basedOn: "Normal", next: "Normal", quickFormat: true,
        run: { size: 36, bold: true, font: "Arial", color: HEADER_FG },
        paragraph: { spacing: { before: 360, after: 120 }, outlineLevel: 0 } },
      { id: "Heading2", name: "Heading 2", basedOn: "Normal", next: "Normal", quickFormat: true,
        run: { size: 28, bold: true, font: "Arial", color: MID_BLUE },
        paragraph: { spacing: { before: 280, after: 80 }, outlineLevel: 1 } },
      { id: "Heading3", name: "Heading 3", basedOn: "Normal", next: "Normal", quickFormat: true,
        run: { size: 24, bold: true, font: "Arial", color: ACCENT },
        paragraph: { spacing: { before: 200, after: 60 }, outlineLevel: 2 } },
    ]
  },
  sections: [{
    properties: {
      page: {
        size: { width: 12240, height: 15840 },
        margin: { top: 1440, right: 1080, bottom: 1440, left: 1080 },
      }
    },
    headers: {
      default: new Header({
        children: [new Paragraph({
          children: [
            new TextRun({ text: "Advanced Programming Techniques with Certo", font: "Arial", size: 18, color: "888888" }),
            new TextRun({ text: "\t", font: "Arial", size: 18 }),
            new TextRun({ text: "Page ", font: "Arial", size: 18, color: "888888" }),
            new TextRun({ children: [PageNumber.CURRENT], font: "Arial", size: 18, color: "888888" }),
          ],
          tabStops: [{ type: "right", position: 9360 }],
          border: { bottom: { style: BorderStyle.SINGLE, size: 4, color: "CCCCCC" } },
        })]
      })
    },
    footers: {
      default: new Footer({
        children: [new Paragraph({
          children: [new TextRun({ text: "© 2025 Certo Language Project  —  Confidential", font: "Arial", size: 16, color: "AAAAAA" })],
          alignment: AlignmentType.CENTER,
          border: { top: { style: BorderStyle.SINGLE, size: 4, color: "CCCCCC" } },
        })]
      })
    },
    children: [

      // ── Cover ──────────────────────────────────────────────────────────────
      new Paragraph({
        children: [],
        spacing: { before: 2400, after: 0 },
      }),
      new Paragraph({
        children: [new TextRun({ text: "Advanced Programming", font: "Arial", size: 72, bold: true, color: BLUE })],
        alignment: AlignmentType.CENTER,
        spacing: { before: 0, after: 0 },
      }),
      new Paragraph({
        children: [new TextRun({ text: "Techniques with Certo", font: "Arial", size: 72, bold: true, color: BLUE })],
        alignment: AlignmentType.CENTER,
        spacing: { before: 0, after: 240 },
      }),
      new Paragraph({
        children: [new TextRun({ text: "Generics · Lambdas · Pattern Matching · Database Integration", font: "Arial", size: 28, color: MID_BLUE, italics: true })],
        alignment: AlignmentType.CENTER,
        spacing: { before: 0, after: 1200 },
      }),
      new Paragraph({
        children: [new TextRun({ text: "Version 1.0  —  June 2025", font: "Arial", size: 22, color: "888888" })],
        alignment: AlignmentType.CENTER,
        spacing: { before: 0, after: 0 },
      }),

      pageBreak(),

      // ── Table of Contents ──────────────────────────────────────────────────
      h1("Table of Contents"),
      new TableOfContents("", { hyperlink: true, headingStyleRange: "1-3" }),

      pageBreak(),

      // ══════════════════════════════════════════════════════════════════════
      // CHAPTER 1 — LANGUAGE FUNDAMENTALS
      // ══════════════════════════════════════════════════════════════════════
      h1("1. Language Fundamentals", "ch1"),
      para("Certo is a statically-typed, expression-oriented language that compiles to C and then to native binaries via Clang. Every value has a type known at compile time; every expression produces a value. This chapter covers the building blocks you need before exploring advanced patterns."),

      h2("1.1 Modules and Imports", "s1_1"),
      para("Every Certo source file begins with a module declaration. Modules are the unit of compilation and namespace isolation."),
      ...code([
        "module Orders",
        "",
        "import Stdlib.Core",
        "import Stdlib.Db",
        "import Math          // local module",
      ]),
      para("Declarations marked pub are exported; unmarked declarations are private to the module."),
      ...code([
        "pub fn calculate(n: Int): Int = n * 2   // visible to importers",
        "fn helper(n: Int): Int = n + 1           // module-private",
      ]),

      h2("1.2 Bindings: val, var, and let", "s1_2"),
      para("Certo has three binding forms:"),
      bullet("val  — immutable binding (preferred). Cannot be reassigned."),
      bullet("let  — alias for val. Familiar to Rust/Swift developers."),
      bullet("var  — mutable binding. Use sparingly."),
      ...code([
        "val pi = 3.14159",
        "let greeting = \"Hello\"      // same as val",
        "var counter = 0",
        "counter = counter + 1       // OK: var is mutable",
        "// pi = 3.0                 // ERROR: val is immutable",
      ]),

      h2("1.3 Types at a Glance", "s1_3"),
      ...code([
        "Int      // 64-bit signed integer",
        "Float    // 64-bit double",
        "Bool     // true / false",
        "Text     // UTF-8 string (immutable)",
        "Unit     // absence of value  (like void)",
        "Int?     // Option<Int>  — either Some(n) or None",
        "List<T>  // dynamic array of T",
        "Map<K,V> // hash map",
      ]),

      pageBreak(),

      // ══════════════════════════════════════════════════════════════════════
      // CHAPTER 2 — FUNCTIONS
      // ══════════════════════════════════════════════════════════════════════
      h1("2. Functions", "ch2"),

      h2("2.1 Function Syntax", "s2_1"),
      para("Functions are declared with fn, a parameter list, an optional return type, and optionally an effect annotation in square brackets. The body is either a single expression or a block."),
      ...code([
        "// Single-expression form",
        "fn square(n: Int): Int = n * n",
        "",
        "// Block form",
        "fn greet(name: Text): Text = {",
        "    let msg = f\"Hello, {name}!\"",
        "    msg",
        "}",
        "",
        "// Effect annotation: [io] means the function performs I/O",
        "fn printSquare(n: Int): Unit [io] = {",
        "    println(intToText(square(n)))",
        "}",
      ]),

      h2("2.2 Named and Default Arguments", "s2_2"),
      para("Call sites can use named arguments for clarity:"),
      ...code([
        "fn createUser(name: Text, age: Int, active: Bool): Text =",
        "    f\"{name} (age {intToText(age)}, active: {boolToText(active)})\"",
        "",
        "// Positional",
        "val a = createUser(\"Alice\", 30, true)",
        "",
        "// Named — order does not matter",
        "val b = createUser(name: \"Bob\", active: false, age: 25)",
      ]),

      h2("2.3 Recursive Functions", "s2_3"),
      para("Certo supports direct recursion. Tail-recursive functions are naturally efficient because the compiler targets C, which eliminates tail calls on optimised builds."),
      ...code([
        "fn factorial(n: Int): Int =",
        "    if n <= 1 then 1 else n * factorial(n - 1)",
        "",
        "fn fibonacci(n: Int): Int =",
        "    if n <= 1 then n else fibonacci(n - 1) + fibonacci(n - 2)",
        "",
        "fn gcd(a: Int, b: Int): Int =",
        "    if b == 0 then a else gcd(b, a % b)",
      ]),

      h2("2.4 Higher-Order Functions", "s2_4"),
      para("Functions are first-class values. You can pass them to other functions, store them in variables, and compose them."),
      ...code([
        "fn applyTwice(f: Int => Int, x: Int): Int = f(f(x))",
        "",
        "fn addN(n: Int): Int => Int = fn(x: Int): Int = x + n",
        "",
        "fn main(): Unit [io] = {",
        "    let double = fn(x: Int): Int = x * 2",
        "    println(intToText(applyTwice(double, 3)))   // 12",
        "",
        "    let add5 = addN(5)",
        "    println(intToText(add5(10)))                // 15",
        "}",
      ]),

      pageBreak(),

      // ══════════════════════════════════════════════════════════════════════
      // CHAPTER 3 — LAMBDAS & TRAILING LAMBDAS
      // ══════════════════════════════════════════════════════════════════════
      h1("3. Lambdas and Trailing Lambdas", "ch3"),

      para("Lambdas are anonymous functions written inline. Certo supports two syntactic forms: the classic inline lambda and the ergonomic trailing lambda block."),

      h2("3.1 Inline Lambdas", "s3_1"),
      para("The inline lambda uses the fat-arrow syntax:"),
      ...code([
        "// fn(param: Type): ReturnType = body",
        "val square = fn(n: Int): Int = n * n",
        "",
        "// Multi-parameter",
        "val add = fn(a: Int, b: Int): Int = a + b",
        "",
        "// Passed directly to a higher-order function",
        "val nums = [1, 2, 3, 4, 5]",
        "val squares = List.map(nums, fn(n: Int): Int = n * n)",
      ]),

      h2("3.2 Trailing Lambda Syntax", "s3_2"),
      para("When the last argument to a function is a lambda, you can move it outside the parentheses into a block. This makes iterator-heavy code read like a built-in language construct."),

      sideBySide(
        "Without trailing lambda",
        "With trailing lambda",
        [
          "val doubled = List.map(nums,",
          "    fn(n: Int): Int = n * 2)",
          "",
          "val evens = List.filter(nums,",
          "    fn(n: Int): Bool = n % 2 == 0)",
          "",
          "val sum = List.fold(nums, 0,",
          "    fn(acc: Int, n: Int): Int = acc + n)",
        ],
        [
          "val doubled = List.map(nums) { n => n * 2 }",
          "",
          "",
          "val evens = List.filter(nums) { n => n % 2 == 0 }",
          "",
          "",
          "val sum = List.fold(nums, 0) { acc, n => acc + n }",
        ]
      ),

      spacer(),
      para("If the lambda is the only argument, parentheses can be omitted entirely:"),
      ...code([
        "// f { params => body }  — no parens needed",
        "val processed = transform { x => x * x + 1 }",
      ]),

      h2("3.3 Multi-Parameter Trailing Lambdas", "s3_3"),
      para("Separate multiple parameters with commas inside the block:"),
      ...code([
        "val result = List.fold(items, []) { acc, item =>",
        "    if item.active then List.push(acc, item) else acc",
        "}",
      ]),

      h2("3.4 Lambdas with Collections", "s3_4"),
      para("The standard collection operations all accept lambdas. This enables a functional pipeline style:"),
      ...code([
        "module Reports",
        "import Stdlib.Core",
        "",
        "type Product = { name: Text, price: Float, inStock: Bool }",
        "",
        "fn main(): Unit [io] = {",
        "    val products = [",
        "        Product { name: \"Widget\",   price: 9.99,  inStock: true  },",
        "        Product { name: \"Gadget\",   price: 24.99, inStock: false },",
        "        Product { name: \"Doohickey\",price: 4.49,  inStock: true  },",
        "    ]",
        "",
        "    // Filter then map using trailing lambdas",
        "    val available = List.filter(products) { p => p.inStock }",
        "    val names     = List.map(available)   { p => p.name }",
        "",
        "    for name in names {",
        "        println(name)",
        "    }",
        "",
        "    // Fold to compute total",
        "    val total = List.fold(available, 0.0) { acc, p => acc + p.price }",
        "    println(f\"Total in-stock value: {floatToText(total)}\")",
        "}",
      ]),

      note("Lambdas in Certo are lifted to top-level C functions by the compiler. They do not capture variables from the enclosing scope (no closures yet). Pass extra context as explicit parameters."),

      pageBreak(),

      // ══════════════════════════════════════════════════════════════════════
      // CHAPTER 4 — GENERICS
      // ══════════════════════════════════════════════════════════════════════
      h1("4. Generics", "ch4"),
      para("Generics let you write functions and types that work over any type while preserving full static type-checking. Certo uses angle-bracket syntax familiar from Rust, Java, and Swift."),

      h2("4.1 Generic Functions", "s4_1"),
      para("Declare type parameters in angle brackets after the function name:"),
      ...code([
        "// Single type parameter",
        "fn identity<T>(x: T): T = x",
        "",
        "// Multiple type parameters",
        "fn pair<A, B>(a: A, b: B): (A, B) = (a, b)",
        "",
        "// Usage — types are inferred at the call site",
        "val n = identity(42)        // T = Int",
        "val s = identity(\"hello\")   // T = Text",
        "val p = pair(1, true)       // A = Int, B = Bool",
      ]),

      h2("4.2 Generic Functions with Collections", "s4_2"),
      para("The most common use of generics is operating on lists without knowing the element type:"),
      ...code([
        "fn first<T>(xs: List<T>): T? = List.first(xs)",
        "",
        "fn last<T>(xs: List<T>): T? =",
        "    if List.len(xs) == 0 then None",
        "    else Some(List.get(xs, List.len(xs) - 1))",
        "",
        "fn isEmpty<T>(xs: List<T>): Bool = List.len(xs) == 0",
        "",
        "fn contains<T>(xs: List<T>, target: T): Bool =",
        "    List.len(List.filter(xs) { x => x == target }) > 0",
      ]),

      h2("4.3 Generic Types", "s4_3"),
      para("User-defined record types can also be parameterised:"),
      ...code([
        "type Box<T> = { value: T, label: Text }",
        "",
        "type Pair<A, B> = { first: A, second: B }",
        "",
        "type Stack<T> = { items: List<T>, size: Int }",
        "",
        "fn main(): Unit [io] = {",
        "    let intBox  = Box { value: 42,        label: \"answer\" }",
        "    let textBox = Box { value: \"Certo\",   label: \"lang\" }",
        "",
        "    let coords = Pair { first: 10, second: 20 }",
        "    println(intToText(coords.first))",
        "}",
      ]),

      h2("4.4 Generic Algorithms", "s4_4"),
      para("Combining generics with lambdas produces reusable algorithms that work on any type:"),
      ...code([
        "// Generic binary search — returns index or None",
        "fn binarySearch<T>(xs: List<T>, target: T, cmp: (T, T) => Int): Int? = {",
        "    var lo = 0",
        "    var hi = List.len(xs) - 1",
        "    var result: Int? = None",
        "    // iterative version using while",
        "    while lo <= hi {",
        "        val mid = (lo + hi) / 2",
        "        val c = cmp(List.get(xs, mid), target)",
        "        if c == 0 then {",
        "            result = Some(mid)",
        "            lo = hi + 1    // break",
        "        } else if c < 0 then",
        "            lo = mid + 1",
        "        else",
        "            hi = mid - 1",
        "    }",
        "    result",
        "}",
        "",
        "// Usage",
        "val sorted = [1, 3, 5, 7, 9, 11]",
        "val idx = binarySearch(sorted, 7) { a, b => a - b }",
        "// idx = Some(3)",
      ]),

      h2("4.5 Option<T> and Result<T, E>", "s4_5"),
      para("Option and Result are the canonical generic types for handling absence and errors without exceptions."),
      ...code([
        "// Option — a value that may or may not be present",
        "val maybePort: Int? = parseInt(arg(1) ?? \"\")",
        "",
        "// Unwrap with a default using ??",
        "val port = maybePort ?? 8080",
        "",
        "// Result — either Ok(value) or Err(message)",
        "fn parsePositive(s: Text): Result<Int, Text> =",
        "    match parseInt(s) {",
        "        Some(n) => if n > 0 then Ok(n) else Err(\"must be positive\")",
        "        None    => Err(f\"not a number: {s}\")",
        "    }",
      ]),

      note("T? is syntactic sugar for Option<T>. The null-coalescing operator ?? unwraps an Option, substituting the right-hand side when the value is None."),

      pageBreak(),

      // ══════════════════════════════════════════════════════════════════════
      // CHAPTER 5 — PATTERN MATCHING
      // ══════════════════════════════════════════════════════════════════════
      h1("5. Pattern Matching", "ch5"),
      para("Certo's match expression is exhaustive: the compiler verifies every case is handled. Patterns can destructure values, bind names, and filter with guards."),

      h2("5.1 match Expression", "s5_1"),
      ...code([
        "fn describe(n: Int): Text =",
        "    match n {",
        "        0 => \"zero\"",
        "        1 => \"one\"",
        "        _ => if n < 0 then \"negative\" else \"large\"",
        "    }",
        "",
        "// Matching on Option",
        "fn showFirst(xs: List<Text>): Text =",
        "    match List.first(xs) {",
        "        Some(s) => f\"First: {s}\"",
        "        None    => \"empty list\"",
        "    }",
        "",
        "// Matching on Result",
        "fn runQuery(conn: Int, sql: Text): Text =",
        "    match dbQuery(conn, sql, []) {",
        "        Ok(rows)  => f\"{intToText(List.len(rows))} rows\"",
        "        Err(msg)  => f\"Error: {msg}\"",
        "    }",
      ]),

      h2("5.2 Guards", "s5_2"),
      para("Add an if condition to an arm to narrow the match:"),
      ...code([
        "fn classify(n: Int): Text =",
        "    match n {",
        "        n if n < 0   => \"negative\"",
        "        0            => \"zero\"",
        "        n if n < 10  => \"small\"",
        "        n if n < 100 => \"medium\"",
        "        _            => \"large\"",
        "    }",
      ]),

      h2("5.3 if let: Single-Arm Match Shorthand", "s5_3"),
      para("When you only care about one variant, if let avoids boilerplate:"),

      sideBySide(
        "match (verbose)",
        "if let (concise)",
        [
          "match parseInt(arg(1) ?? \"\") {",
          "    Some(n) => {",
          "        println(intToText(n * 2))",
          "    }",
          "    _ => unit",
          "}",
        ],
        [
          "if let Some(n) = parseInt(arg(1) ?? \"\") {",
          "    println(intToText(n * 2))",
          "}",
          "",
          "",
          "",
        ]
      ),

      spacer(),
      para("if let also supports an else branch:"),
      ...code([
        "// From the msgbox example",
        "val title = if let Some(t) = arg(1) { t } else { \"Message\" }",
        "val body  = if let Some(m) = arg(2) { m } else { \"Hello!\" }",
      ]),

      h2("5.4 Destructuring Records", "s5_4"),
      ...code([
        "type Point = { x: Int, y: Int }",
        "",
        "fn quadrant(p: Point): Text =",
        "    match (p.x > 0, p.y > 0) {",
        "        (true,  true)  => \"I\"",
        "        (false, true)  => \"II\"",
        "        (false, false) => \"III\"",
        "        (true,  false) => \"IV\"",
        "    }",
      ]),

      pageBreak(),

      // ══════════════════════════════════════════════════════════════════════
      // CHAPTER 6 — RECORDS AND STRUCT SPREAD
      // ══════════════════════════════════════════════════════════════════════
      h1("6. Records and Struct Spread", "ch6"),

      h2("6.1 Defining Record Types", "s6_1"),
      para("Records are product types: named collections of typed fields."),
      ...code([
        "type User = {",
        "    id:     Int",
        "    name:   Text",
        "    email:  Text",
        "    active: Bool",
        "}",
        "",
        "type Address = {",
        "    street: Text",
        "    city:   Text",
        "    zip:    Text",
        "}",
        "",
        "type Customer = {",
        "    user:    User",
        "    address: Address",
        "    credit:  Float",
        "}",
      ]),

      h2("6.2 Creating and Accessing Records", "s6_2"),
      ...code([
        "val alice = User {",
        "    id:     1",
        "    name:   \"Alice\"",
        "    email:  \"alice@example.com\"",
        "    active: true",
        "}",
        "",
        "println(alice.name)           // Alice",
        "println(alice.email)          // alice@example.com",
      ]),

      h2("6.3 Struct Spread (Record Update Syntax)", "s6_3"),
      para("The .. spread operator copies all fields from an existing record and overrides only the specified ones. This is the idiomatic way to produce updated values without mutation."),

      sideBySide(
        "Without spread (repetitive)",
        "With spread (concise)",
        [
          "val alice2 = User {",
          "    id:     alice.id",
          "    name:   alice.name",
          "    email:  \"new@email.com\"",
          "    active: alice.active",
          "}",
        ],
        [
          "val alice2 = User {",
          "    ..alice",
          "    email: \"new@email.com\"",
          "}",
          "",
          "",
          "",
        ]
      ),

      spacer(),
      para("Spread is especially useful in state-machine patterns where you update a single field of a large record:"),
      ...code([
        "type AppState = {",
        "    user:     User",
        "    page:     Text",
        "    loading:  Bool",
        "    errorMsg: Text",
        "}",
        "",
        "fn navigateTo(state: AppState, page: Text): AppState =",
        "    AppState { ..state, page: page, loading: true, errorMsg: \"\" }",
        "",
        "fn finishLoad(state: AppState): AppState =",
        "    AppState { ..state, loading: false }",
        "",
        "fn setError(state: AppState, msg: Text): AppState =",
        "    AppState { ..state, loading: false, errorMsg: msg }",
      ]),

      pageBreak(),

      // ══════════════════════════════════════════════════════════════════════
      // CHAPTER 7 — COLLECTIONS
      // ══════════════════════════════════════════════════════════════════════
      h1("7. Collections", "ch7"),

      h2("7.1 Lists", "s7_1"),
      para("List<T> is a dynamic array. All standard operations are generic and work with trailing lambdas."),
      ...code([
        "val nums  = [1, 2, 3, 4, 5]",
        "val empty: List<Int> = []",
        "",
        "// Core operations",
        "val len    = List.len(nums)                // 5",
        "val first  = List.first(nums)              // Some(1)",
        "val item   = List.get(nums, 2)             // 3",
        "val pushed = List.push(nums, 6)            // [1,2,3,4,5,6]",
        "",
        "// Transformation",
        "val doubled  = List.map(nums)    { n => n * 2 }",
        "val evens    = List.filter(nums) { n => n % 2 == 0 }",
        "val sum      = List.fold(nums, 0){ acc, n => acc + n }",
        "val sorted   = List.sort(nums)   { a, b => a - b }",
      ]),

      h2("7.2 For Loops", "s7_2"),
      para("The for ... in construct iterates a List. It is sugar for a forEach call and integrates naturally with records:"),
      ...code([
        "type Product = { name: Text, price: Float }",
        "",
        "fn printCatalogue(products: List<Product>): Unit [io] = {",
        "    for p in products {",
        "        println(f\"{p.name}: ${floatToText(p.price)}\")",
        "    }",
        "}",
        "",
        "// Nested loops",
        "fn multiplicationTable(n: Int): Unit [io] = {",
        "    val rows = List.range(1, n + 1)",
        "    for i in rows {",
        "        for j in rows {",
        "            val cell = intToText(i * j)",
        "            print(f\"{cell}\\t\")",
        "        }",
        "        println(\"\")",
        "    }",
        "}",
      ]),

      h2("7.3 Maps", "s7_3"),
      para("Map<K, V> is an associative hash map."),
      ...code([
        "val scores: Map<Text, Int> = Map.empty()",
        "val s1 = Map.insert(scores, \"Alice\", 95)",
        "val s2 = Map.insert(s1,     \"Bob\",   87)",
        "",
        "val aliceScore = Map.get(s2, \"Alice\")  // Some(95)",
        "val carol      = Map.get(s2, \"Carol\")  // None",
        "",
        "// ?? to provide a default",
        "val safeScore = Map.get(s2, \"Carol\") ?? 0",
        "",
        "val keys = Map.keys(s2)      // List<Text>",
        "val vals = Map.values(s2)    // List<Int>",
      ]),

      h2("7.4 Building Maps from Lists", "s7_4"),
      para("A common pattern is building a lookup map from a list using fold:"),
      ...code([
        "type User = { id: Int, name: Text }",
        "",
        "fn buildIndex(users: List<User>): Map<Int, User> =",
        "    List.fold(users, Map.empty()) { acc, u => Map.insert(acc, u.id, u) }",
        "",
        "fn main(): Unit [io] = {",
        "    val users = [",
        "        User { id: 1, name: \"Alice\" },",
        "        User { id: 2, name: \"Bob\" },",
        "    ]",
        "    val index = buildIndex(users)",
        "    if let Some(u) = Map.get(index, 1) {",
        "        println(u.name)    // Alice",
        "    }",
        "}",
      ]),

      pageBreak(),

      // ══════════════════════════════════════════════════════════════════════
      // CHAPTER 8 — STRING INTERPOLATION
      // ══════════════════════════════════════════════════════════════════════
      h1("8. String Interpolation", "ch8"),

      para("Certo supports f-strings (format strings) for embedding expressions directly in text literals. The prefix f before a quoted string enables interpolation."),

      h2("8.1 Basic Interpolation", "s8_1"),
      ...code([
        "val name  = \"Alice\"",
        "val age   = 30",
        "val msg   = f\"Hello, {name}! You are {intToText(age)} years old.\"",
        "// -> \"Hello, Alice! You are 30 years old.\"",
      ]),

      h2("8.2 Expressions Inside Braces", "s8_2"),
      para("Any valid Certo expression can appear inside the braces, including method calls, arithmetic, and conditionals:"),
      ...code([
        "val items  = [\"a\", \"b\", \"c\"]",
        "val report = f\"Found {intToText(List.len(items))} items\"",
        "",
        "val price  = 9.99",
        "val tax    = price * 0.2",
        "val label  = f\"Price: {floatToText(price)}, Tax: {floatToText(tax)}\"",
        "",
        "val status = f\"Status: {if active then \"ON\" else \"OFF\"}\"",
      ]),

      h2("8.3 String Concatenation", "s8_3"),
      para("The ++ operator concatenates two Text values. Use it when building strings from dynamic parts:"),
      ...code([
        "fn buildSql(table: Text, limit: Int): Text =",
        "    \"SELECT * FROM \" ++ table ++ \" LIMIT \" ++ intToText(limit)",
      ]),

      pageBreak(),

      // ══════════════════════════════════════════════════════════════════════
      // CHAPTER 9 — DATABASE INTEGRATION
      // ══════════════════════════════════════════════════════════════════════
      h1("9. Database Integration", "ch9"),
      para("Certo ships with Stdlib.Db, a PostgreSQL client built on libpq. The API is deliberately low-level and explicit: you write SQL, bind parameters, and receive results as lists of text rows. This gives you full control without an ORM magic layer."),

      h2("9.1 Connecting", "s9_1"),
      ...code([
        "import Stdlib.Core",
        "import Stdlib.Db",
        "",
        "fn main(): Unit [io] = {",
        "    val connstr = arg(1) ?? \"host=localhost dbname=mydb user=postgres\"",
        "    val conn = dbConnect(connstr)",
        "",
        "    if conn == 0 then {",
        "        eprintln(f\"connection failed: {dbError(0)}\")",
        "    } else {",
        "        // ... work with conn",
        "        dbClose(conn)",
        "    }",
        "}",
      ]),

      h2("9.2 Parameterized Queries", "s9_2"),
      para("Always use parameterized queries. Parameters are passed as a List<Text> and bound as $1, $2, ... in the SQL. This prevents SQL injection and handles type conversions automatically."),
      ...code([
        "// INSERT with parameters",
        "dbExec(conn,",
        "    \"INSERT INTO users(name, email, age) VALUES ($1, $2, $3)\" ++",
        "    \" ON CONFLICT (email) DO NOTHING\",",
        "    [\"Alice\", \"alice@example.com\", \"30\"])",
        "",
        "// SELECT with a parameter",
        "val minAge = \"28\"",
        "val rows = dbQuery(conn,",
        "    \"SELECT name, age FROM users WHERE age >= $1 ORDER BY age\",",
        "    [minAge])",
      ]),

      h2("9.3 Reading Results", "s9_3"),
      para("dbQuery returns List<List<Text>>  — a list of rows, each row being a list of column values as text. dbColumns returns the column names."),
      ...code([
        "val cols = dbColumns(conn, \"SELECT id, name, email FROM users\")",
        "val rows = dbQuery(conn,   \"SELECT id, name, email FROM users\", [])",
        "",
        "// Print header",
        "val header = List.fold(cols, \"\") { acc, c =>",
        "    if acc == \"\" then c else acc ++ \" | \" ++ c",
        "}",
        "println(header)",
        "",
        "// Print rows",
        "for row in rows {",
        "    val line = List.fold(row, \"\") { acc, cell =>",
        "        if acc == \"\" then cell else acc ++ \" | \" ++ cell",
        "    }",
        "    println(line)",
        "}",
      ]),

      h2("9.4 Transactions", "s9_4"),
      para("Wrap multiple writes in a transaction to guarantee atomicity. dbBegin / dbCommit / dbRollback follow the standard begin-commit-rollback model."),
      ...code([
        "fn transferFunds(",
        "    conn: Int, fromId: Int, toId: Int, amount: Float",
        "): Bool [io] = {",
        "    dbBegin(conn)",
        "",
        "    val ok1 = dbExec(conn,",
        "        \"UPDATE accounts SET balance = balance - $1 WHERE id = $2\",",
        "        [floatToText(amount), intToText(fromId)])",
        "",
        "    val ok2 = dbExec(conn,",
        "        \"UPDATE accounts SET balance = balance + $1 WHERE id = $2\",",
        "        [floatToText(amount), intToText(toId)])",
        "",
        "    if ok1 and ok2 then {",
        "        dbCommit(conn)",
        "        true",
        "    } else {",
        "        dbRollback(conn)",
        "        false",
        "    }",
        "}",
      ]),

      h2("9.5 Schema Migrations", "s9_5"),
      para("Use dbExec to run DDL statements. Wrapping them in transactions makes migrations atomic and reversible:"),
      ...code([
        "fn migrate(conn: Int): Unit [io] = {",
        "    dbBegin(conn)",
        "",
        "    dbExec(conn, \"CREATE TABLE IF NOT EXISTS schema_versions (\",  [])",
        "    dbExec(conn, \"    version INT PRIMARY KEY,\",                   [])",
        "    dbExec(conn, \"    applied_at TIMESTAMPTZ DEFAULT NOW()\",        [])",
        "    dbExec(conn, \")\",                                               [])",
        "",
        "    val applied = dbQuery(conn,",
        "        \"SELECT version FROM schema_versions ORDER BY version\", [])",
        "",
        "    // Only run each migration once",
        "    if List.len(applied) < 1 then {",
        "        dbExec(conn,",
        "            \"ALTER TABLE orders ADD COLUMN notes TEXT DEFAULT ''\", [])",
        "        dbExec(conn,",
        "            \"INSERT INTO schema_versions(version) VALUES (1)\", [])",
        "        println(\"Applied migration 1\")",
        "    }",
        "",
        "    dbCommit(conn)",
        "}",
      ]),

      h2("9.6 Clever DB Patterns", "s9_6"),

      h3("Bulk Insert with a Loop"),
      ...code([
        "fn bulkInsert(conn: Int, items: List<Text>): Unit [io] = {",
        "    dbBegin(conn)",
        "    for item in items {",
        "        dbExec(conn,",
        "            \"INSERT INTO log(message) VALUES ($1)\",",
        "            [item])",
        "    }",
        "    dbCommit(conn)",
        "}",
      ]),

      h3("Building a Query from Filters"),
      para("Compose SQL dynamically by folding over a list of conditions:"),
      ...code([
        "type Filter = { column: Text, value: Text }",
        "",
        "fn buildWhereClause(filters: List<Filter>): (Text, List<Text>) = {",
        "    var idx = 1",
        "    var clauses: List<Text> = []",
        "    var params:  List<Text> = []",
        "",
        "    for f in filters {",
        "        clauses = List.push(clauses, f.column ++ \" = $\" ++ intToText(idx))",
        "        params  = List.push(params, f.value)",
        "        idx = idx + 1",
        "    }",
        "",
        "    val where = if List.len(clauses) == 0 then \"\"",
        "                else \" WHERE \" ++ List.join(clauses, \" AND \")",
        "    (where, params)",
        "}",
        "",
        "fn search(conn: Int, filters: List<Filter>): List<List<Text>> [io] = {",
        "    val (where, params) = buildWhereClause(filters)",
        "    dbQuery(conn, \"SELECT * FROM products\" ++ where, params)",
        "}",
      ]),

      h3("Generic Row-to-Record Mapper"),
      para("Use a lambda to turn raw text rows into typed records:"),
      ...code([
        "type User = { id: Int, name: Text, email: Text }",
        "",
        "fn queryUsers(conn: Int): List<User> [io] = {",
        "    val rows = dbQuery(conn,",
        "        \"SELECT id, name, email FROM users ORDER BY id\", [])",
        "",
        "    List.map(rows) { row =>",
        "        User {",
        "            id:    parseInt(List.get(row, 0)) ?? 0",
        "            name:  List.get(row, 1)",
        "            email: List.get(row, 2)",
        "        }",
        "    }",
        "}",
      ]),

      pageBreak(),

      // ══════════════════════════════════════════════════════════════════════
      // CHAPTER 10 — HTTP AND JSON
      // ══════════════════════════════════════════════════════════════════════
      h1("10. HTTP and JSON", "ch10"),

      para("Certo's standard library includes Stdlib.Http for serving and Stdlib.Json for building JSON values."),

      h2("10.1 HTTP Server", "s10_1"),
      ...code([
        "module Api",
        "import Stdlib.Core",
        "import Stdlib.Http",
        "import Stdlib.Json",
        "",
        "fn handler(req: HttpRequest): HttpResponse = {",
        "    val method = HttpRequest.method(req)",
        "    val path   = HttpRequest.path(req)",
        "",
        "    if path == \"/\" then",
        "        Http.ok(\"Hello from Certo!\", \"text/plain\")",
        "",
        "    else if path == \"/health\" then",
        "        Http.ok(\"{\\\"status\\\":\\\"ok\\\"}\", \"application/json\")",
        "",
        "    else if path == \"/echo\" and method == \"POST\" then {",
        "        val body = HttpRequest.body(req)",
        "        val obj  = Json.object()",
        "        JsonValue.set(obj, \"body\", Json.string(body))",
        "        Http.ok(Json.stringify(obj), \"application/json\")",
        "    }",
        "",
        "    else",
        "        Http.notFound(f\"No route: {method} {path}\")",
        "}",
        "",
        "fn main(): Unit [io] = {",
        "    val port = parseInt(arg(1) ?? \"8080\") ?? 8080",
        "    println(f\"Listening on http://localhost:{intToText(port)}\")",
        "    Http.serve(port, handler)",
        "}",
      ]),

      h2("10.2 Building JSON Responses", "s10_2"),
      para("Stdlib.Json provides a builder API for constructing JSON values:"),
      ...code([
        "fn userToJson(u: User): Text = {",
        "    val obj = Json.object()",
        "    JsonValue.set(obj, \"id\",    Json.int(u.id))",
        "    JsonValue.set(obj, \"name\",  Json.string(u.name))",
        "    JsonValue.set(obj, \"email\", Json.string(u.email))",
        "    Json.stringify(obj)",
        "}",
        "",
        "fn usersToJson(users: List<User>): Text = {",
        "    val arr = Json.array()",
        "    for u in users {",
        "        val obj = Json.object()",
        "        JsonValue.set(obj, \"id\",   Json.int(u.id))",
        "        JsonValue.set(obj, \"name\", Json.string(u.name))",
        "        Json.arrayPush(arr, obj)",
        "    }",
        "    Json.stringify(arr)",
        "}",
      ]),

      pageBreak(),

      // ══════════════════════════════════════════════════════════════════════
      // CHAPTER 11 — ADVANCED PATTERNS
      // ══════════════════════════════════════════════════════════════════════
      h1("11. Advanced Patterns", "ch11"),

      h2("11.1 Pipeline Style with |>", "s11_1"),
      para("The pipe operator |> passes the left-hand value as the first argument of the right-hand function. It enables a left-to-right reading order that mirrors data flow:"),
      ...code([
        "// Without pipe",
        "val result = List.map(List.filter(List.sort(nums) { a, b => a - b }) { n => n > 2 }) { n => n * 10 }",
        "",
        "// With pipe — reads top to bottom like a pipeline",
        "val result =",
        "    nums",
        "    |> List.sort    { a, b => a - b }",
        "    |> List.filter  { n => n > 2 }",
        "    |> List.map     { n => n * 10 }",
      ]),

      h2("11.2 Guard Expressions", "s11_2"),
      para("guard is an early-return construct for pre-condition checks. It reads as \"ensure this is true, otherwise return the else expression\":"),
      ...code([
        "fn safeDivide(a: Int, b: Int): Int? = {",
        "    guard b != 0 else None",
        "    Some(a / b)",
        "}",
        "",
        "fn processOrder(order: Order): Text = {",
        "    guard order.items > 0  else \"empty order\"",
        "    guard order.paid       else \"payment required\"",
        "    guard order.inStock    else \"out of stock\"",
        "    \"order confirmed\"",
        "}",
      ]),

      h2("11.3 Error Propagation with ?", "s11_3"),
      para("The ? operator on a Result<T, E> either unwraps the Ok value or returns the Err early from the enclosing function. It is identical to Rust's ? operator:"),
      ...code([
        "fn parsePort(s: Text): Result<Int, Text> =",
        "    match parseInt(s) {",
        "        Some(n) => if n > 0 and n < 65536 then Ok(n) else Err(\"out of range\")",
        "        None    => Err(f\"not a number: {s}\")",
        "    }",
        "",
        "fn startServer(portStr: Text): Result<Unit, Text> [io] = {",
        "    val port = parsePort(portStr)?   // propagate Err if parsePort fails",
        "    Http.serve(port, handler)",
        "    Ok(unit)",
        "}",
      ]),

      h2("11.4 Combining Generics, Lambdas, and DB", "s11_4"),
      para("These features compose naturally. Here is a complete pattern that queries the database, maps rows to typed records using a generic mapper function, and returns filtered results:"),
      ...code([
        "module Reporting",
        "import Stdlib.Core",
        "import Stdlib.Db",
        "",
        "type SalesRow = { region: Text, product: Text, revenue: Float }",
        "",
        "fn fetchSales(conn: Int, minRevenue: Float): List<SalesRow> [io] = {",
        "    val rows = dbQuery(conn,",
        "        \"SELECT region, product, revenue FROM sales\" ++",
        "        \" WHERE revenue >= $1 ORDER BY revenue DESC\",",
        "        [floatToText(minRevenue)])",
        "",
        "    // Map raw text rows to typed records",
        "    List.map(rows) { row =>",
        "        SalesRow {",
        "            region:  List.get(row, 0)",
        "            product: List.get(row, 1)",
        "            revenue: parseFloat(List.get(row, 2)) ?? 0.0",
        "        }",
        "    }",
        "}",
        "",
        "fn topRegions(sales: List<SalesRow>): List<Text> =",
        "    sales",
        "    |> List.map    { s => s.region }",
        "    |> List.sort   { a, b => if a < b then -1 else if a > b then 1 else 0 }",
        "    |> List.filter { r => r != \"\" }",
        "",
        "fn main(): Unit [io] = {",
        "    val conn  = dbConnect(arg(1) ?? \"host=localhost dbname=sales\")",
        "    val sales = fetchSales(conn, 1000.0)",
        "",
        "    println(f\"High-value sales: {intToText(List.len(sales))}\")",
        "    for s in sales {",
        "        println(f\"{s.region}: {s.product} — ${floatToText(s.revenue)}\")",
        "    }",
        "",
        "    let regions = topRegions(sales)",
        "    println(\"\\nRegions:\")",
        "    for r in regions { println(f\"  {r}\") }",
        "",
        "    dbClose(conn)",
        "}",
      ]),

      pageBreak(),

      // ══════════════════════════════════════════════════════════════════════
      // CHAPTER 12 — BUILDING AND RUNNING
      // ══════════════════════════════════════════════════════════════════════
      h1("12. Building and Running", "ch12"),

      h2("12.1 The Certo CLI", "s12_1"),
      para("Certo ships as a single executable. The primary subcommand is build:"),
      ...code([
        "# Compile to a native executable",
        "certo build myapp.cto",
        "",
        "# Specify output file",
        "certo build myapp.cto -o dist/myapp",
        "",
        "# Emit the intermediate C source (useful for debugging)",
        "certo build myapp.cto --emit-c",
        "",
        "# Build a shared library (.dll / .so)",
        "certo build mylib.cto --emit-dll",
      ]),

      h2("12.2 Passing Arguments to Programs", "s12_2"),
      para("The stdlib arg(n) function returns the nth command-line argument as Text?. Certo uses a Unicode-aware entry point on Windows (wmain) so quoted strings with spaces work correctly from any shell:"),
      ...code([
        "// Access command-line arguments",
        "val name    = arg(1) ?? \"World\"",
        "val verbose = arg(2) == Some(\"--verbose\")",
        "",
        "// Running the program",
        "// myapp.exe \"Hello World\" --verbose",
        "// arg(0) = path to exe",
        "// arg(1) = \"Hello World\"   (the space is preserved)",
        "// arg(2) = \"--verbose\"",
      ]),

      h2("12.3 Effect System", "s12_3"),
      para("Effect annotations in square brackets document what a function does beyond computing a value:"),
      bullet("[io]   — performs I/O (print, read, file system, network, DB)"),
      bullet("[async]— returns a future / may await"),
      para("Functions without an effect annotation are pure: given the same arguments they always return the same value. This is enforced by convention today and will be checked by the type system in a future version."),

      pageBreak(),

      // ══════════════════════════════════════════════════════════════════════
      // CHAPTER 13 — QUICK REFERENCE
      // ══════════════════════════════════════════════════════════════════════
      h1("13. Quick Reference", "ch13"),

      h2("13.1 Operator Summary", "s13_1"),

      (() => {
        const border = { style: BorderStyle.SINGLE, size: 1, color: "CCCCCC" };
        const borders = { top: border, bottom: border, left: border, right: border };
        const cm = { top: 60, bottom: 60, left: 120, right: 120 };
        const hdrBg = MID_BLUE;
        const altBg = "EEF4FB";
        const whiteBg = "FFFFFF";

        const rows_data = [
          ["Operator", "Meaning", "Example"],
          ["??", "Null-coalesce (unwrap Option)", "x ?? 0"],
          ["|>", "Pipe: pass left as first arg", "nums |> List.map { n => n*2 }"],
          ["++", "Text concatenation", "\"Hello\" ++ name"],
          ["?", "Propagate Err (Result)", "parsePort(s)?"],
          ["..", "Struct spread / record update", "User { ..u, name: \"Bob\" }"],
          ["=>", "Lambda / match arm", "fn(x) => x + 1"],
          ["and / or", "Boolean logic", "a > 0 and b > 0"],
          ["==  !=", "Equality", "x == 42"],
          ["**", "Exponentiation", "2 ** 10"],
        ];

        return new Table({
          width: { size: 9360, type: WidthType.DXA },
          columnWidths: [1440, 3960, 3960],
          rows: rows_data.map((row, ri) =>
            new TableRow({
              tableHeader: ri === 0,
              children: row.map((cell, ci) =>
                new TableCell({
                  borders,
                  margins: cm,
                  width: { size: [1440, 3960, 3960][ci], type: WidthType.DXA },
                  shading: { fill: ri === 0 ? hdrBg : ri % 2 === 0 ? altBg : whiteBg, type: ShadingType.CLEAR },
                  verticalAlign: VerticalAlign.CENTER,
                  children: [new Paragraph({
                    children: [new TextRun({
                      text: cell,
                      font: ci === 0 ? "Courier New" : "Arial",
                      size: ci === 0 ? 18 : 20,
                      bold: ri === 0,
                      color: ri === 0 ? HEADER_FG : "1A1A1A",
                    })]
                  })]
                })
              )
            })
          )
        });
      })(),

      spacer(),

      h2("13.2 Stdlib Functions Cheat Sheet", "s13_2"),

      (() => {
        const border = { style: BorderStyle.SINGLE, size: 1, color: "CCCCCC" };
        const borders = { top: border, bottom: border, left: border, right: border };
        const cm = { top: 60, bottom: 60, left: 120, right: 120 };
        const altBg = "F5F5F5";

        const groups = [
          ["Core",   ["println(t)","eprintln(t)","intToText(n)","floatToText(f)","boolToText(b)","parseInt(t)","parseFloat(t)","arg(n)","argCount()"]],
          ["List",   ["List.len(l)","List.first(l)","List.last(l)","List.get(l,i)","List.push(l,x)","List.map(l){f}","List.filter(l){f}","List.fold(l,z){f}","List.sort(l){cmp}","List.range(lo,hi)"]],
          ["Map",    ["Map.empty()","Map.insert(m,k,v)","Map.get(m,k)","Map.contains(m,k)","Map.remove(m,k)","Map.keys(m)","Map.values(m)","Map.len(m)"]],
          ["Db",     ["dbConnect(connstr)","dbClose(conn)","dbExec(c,sql,params)","dbQuery(c,sql,params)","dbColumns(c,sql)","dbBegin(c)","dbCommit(c)","dbRollback(c)","dbError(c)"]],
          ["Json",   ["Json.object()","Json.array()","Json.string(t)","Json.int(n)","Json.float(f)","Json.bool(b)","JsonValue.set(o,k,v)","Json.arrayPush(a,v)","Json.stringify(v)"]],
          ["Http",   ["Http.serve(port,handler)","Http.ok(body,ct)","Http.notFound(msg)","Http.badRequest(msg)","HttpRequest.method(r)","HttpRequest.path(r)","HttpRequest.body(r)","HttpRequest.header(r,k)"]],
        ];

        const allRows = [];
        groups.forEach(([group, fns]) => {
          // Group header
          allRows.push(new TableRow({
            children: [
              new TableCell({
                borders, margins: cm,
                width: { size: 9360, type: WidthType.DXA },
                columnSpan: 2,
                shading: { fill: LIGHT_BLUE, type: ShadingType.CLEAR },
                children: [new Paragraph({ children: [new TextRun({ text: group, bold: true, font: "Arial", size: 20, color: BLUE })] })]
              })
            ]
          }));
          // Functions in pairs
          for (let i = 0; i < fns.length; i += 2) {
            const left  = fns[i]   || "";
            const right = fns[i+1] || "";
            const rowIdx = Math.floor(i / 2);
            allRows.push(new TableRow({
              children: [left, right].map((fn, ci) =>
                new TableCell({
                  borders, margins: cm,
                  width: { size: 4680, type: WidthType.DXA },
                  shading: { fill: rowIdx % 2 === 0 ? "FFFFFF" : altBg, type: ShadingType.CLEAR },
                  children: [new Paragraph({ children: [new TextRun({ text: fn, font: "Courier New", size: 18 })] })]
                })
              )
            }));
          }
        });

        return new Table({
          width: { size: 9360, type: WidthType.DXA },
          columnWidths: [4680, 4680],
          rows: allRows,
        });
      })(),

      spacer(),

      h2("13.3 Common Patterns at a Glance", "s13_3"),

      ...code([
        "// if let — single-arm match",
        "if let Some(n) = maybeInt { println(intToText(n)) }",
        "",
        "// Struct spread",
        "val updated = MyRecord { ..original, field: newValue }",
        "",
        "// Trailing lambda",
        "val doubled = List.map(nums) { n => n * 2 }",
        "",
        "// Pipeline",
        "nums |> List.filter { n => n > 0 } |> List.map { n => n * n }",
        "",
        "// DB: query + map rows",
        "val users = List.map(dbQuery(conn, sql, [])) { row =>",
        "    User { id: parseInt(List.get(row,0)) ?? 0, name: List.get(row,1) }",
        "}",
        "",
        "// Generic function",
        "fn wrap<T>(x: T): List<T> = [x]",
        "",
        "// Error propagation",
        "val port = parsePort(arg(1) ?? \"8080\")?",
      ]),

      pageBreak(),

      // ── Back matter ────────────────────────────────────────────────────────
      h1("Appendix: Example Programs"),

      h2("A.1 Complete: User Management CLI", "s_a1"),
      para("A full program combining generics, lambdas, pattern matching, and database operations:"),
      ...code([
        "module UserCli",
        "import Stdlib.Core",
        "import Stdlib.Db",
        "",
        "type User = { id: Int, name: Text, email: Text, age: Int }",
        "",
        "fn rowToUser(row: List<Text>): User =",
        "    User {",
        "        id:    parseInt(List.get(row, 0)) ?? 0",
        "        name:  List.get(row, 1)",
        "        email: List.get(row, 2)",
        "        age:   parseInt(List.get(row, 3)) ?? 0",
        "    }",
        "",
        "fn fetchAll(conn: Int): List<User> [io] =",
        "    List.map(",
        "        dbQuery(conn, \"SELECT id,name,email,age FROM users ORDER BY id\", []),",
        "        rowToUser",
        "    )",
        "",
        "fn printUsers(users: List<User>): Unit [io] = {",
        "    println(f\"{intToText(List.len(users))} users:\")",
        "    for u in users {",
        "        println(f\"  [{intToText(u.id)}] {u.name} <{u.email}> age {intToText(u.age)}\")",
        "    }",
        "}",
        "",
        "fn addUser(conn: Int, name: Text, email: Text, age: Int): Unit [io] = {",
        "    dbBegin(conn)",
        "    dbExec(conn,",
        "        \"INSERT INTO users(name,email,age) VALUES ($1,$2,$3)\",",
        "        [name, email, intToText(age)])",
        "    dbCommit(conn)",
        "    println(f\"Added user: {name}\")",
        "}",
        "",
        "fn main(): Unit [io] = {",
        "    let conn = dbConnect(arg(1) ?? \"host=localhost dbname=mydb\")",
        "    guard conn != 0 else {",
        "        eprintln(f\"connect failed: {dbError(0)}\")",
        "    }",
        "",
        "    let cmd = arg(2) ?? \"list\"",
        "",
        "    if cmd == \"list\" then {",
        "        let users = fetchAll(conn)",
        "        printUsers(users)",
        "    } else if cmd == \"add\" then {",
        "        let name  = arg(3) ?? \"\"",
        "        let email = arg(4) ?? \"\"",
        "        let age   = parseInt(arg(5) ?? \"0\") ?? 0",
        "        guard name != \"\" and email != \"\" else {",
        "            eprintln(\"usage: usercli <conn> add <name> <email> <age>\")",
        "        }",
        "        addUser(conn, name, email, age)",
        "    } else {",
        "        eprintln(f\"unknown command: {cmd}\")",
        "    }",
        "",
        "    dbClose(conn)",
        "}",
      ]),

      spacer(),
      para("Compile and run:"),
      ...code([
        "certo build usercli.cto -o usercli",
        "usercli \"host=localhost dbname=mydb\" list",
        "usercli \"host=localhost dbname=mydb\" add Alice alice@example.com 30",
      ]),

    ] // end children
  }] // end sections
});

Packer.toBuffer(doc).then(buf => {
  fs.writeFileSync("Advanced_Programming_Certo.docx", buf);
  console.log("Written: Advanced_Programming_Certo.docx");
}).catch(err => { console.error(err); process.exit(1); });
