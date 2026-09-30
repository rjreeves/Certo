//! C# code generation from compiled QL statements: the typed contract (parameter
//! types, result columns with nullability) becomes C# types, so a host calls
//! `await connection.RecentOrdersAsync(minTotal, since)` and gets
//! `List<RecentOrdersRow>` instead of binding `$1` and reading columns by hand.
//!
//! The output is plain ADO.NET (`System.Data.Common`), so it runs on any
//! provider; it is written for Npgsql (PostgreSQL) and Microsoft.Data.Sqlite
//! (SQLite), which the generated code is tested against.
//!
//! * One `sealed record <Name>Row` per statement with result columns, and one
//!   extension method `<Name>Async` on `DbConnection` per statement.
//!   A mutation without `returning` returns the affected-row count.
//! * Nullable parameters and columns are `T?`; `null` is bound as `DBNull`.
//! * Enums become C# enums, with their database text in a generated mapping.
//! * Every statement's SQL is also exposed as a constant (`<Name>Sql`).
//! * Timestamps are `DateTimeOffset` (`timestamp`) and `DateTime` (`timestamp
//!   naive`), dates `DateOnly`, JSON the JSON text. Under SQLite these are
//!   stored as text in whatever form the driver writes, which is not always
//!   the form `now()` uses: compare timestamps in the application, not in SQL.

use crate::{lower_mutation_with, lower_with, LowerOptions, MutationKind, Statement};
use certo_sdl::{Builtin, SchemaIR, TypeIR};
use certo_sql::Dialect;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write;

#[derive(Debug, Clone)]
pub struct CSharpOptions {
    pub namespace: String,
    /// The static class holding the extension methods.
    pub class_name: String,
    /// The dialect the statements were compiled for: it decides how parameters are named.
    pub dialect: Dialect,
}

impl Default for CSharpOptions {
    fn default() -> Self {
        CSharpOptions { namespace: "Certo.Generated".into(), class_name: "CertoQueries".into(), dialect: Dialect::Postgres }
    }
}

const KEYWORDS: &[&str] = &[
    "abstract", "as", "base", "bool", "break", "byte", "case", "catch", "char", "checked", "class", "const", "continue",
    "decimal", "default", "delegate", "do", "double", "else", "enum", "event", "explicit", "extern", "false", "finally",
    "fixed", "float", "for", "foreach", "goto", "if", "implicit", "in", "int", "interface", "internal", "is", "lock", "long",
    "namespace", "new", "null", "object", "operator", "out", "override", "params", "private", "protected", "public",
    "readonly", "ref", "return", "sbyte", "sealed", "short", "sizeof", "stackalloc", "static", "string", "struct", "switch",
    "this", "throw", "true", "try", "typeof", "uint", "ulong", "unchecked", "unsafe", "ushort", "using", "virtual", "void",
    "volatile", "while",
];

/// `customer_id` -> `CustomerId`. Always a valid identifier.
fn pascal(s: &str) -> String {
    let mut out = String::new();
    for part in s.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()) {
        let mut cs = part.chars();
        if let Some(f) = cs.next() {
            out.extend(f.to_uppercase());
            out.push_str(cs.as_str());
        }
    }
    if out.is_empty() {
        out.push_str("Item");
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// `min_total` -> `minTotal`, escaped if it is a C# keyword.
fn camel(s: &str) -> String {
    let p = pascal(s);
    let mut cs = p.chars();
    let lower: String = match cs.next() {
        Some(f) => f.to_lowercase().chain(cs).collect(),
        None => p.clone(),
    };
    if KEYWORDS.contains(&lower.as_str()) { format!("@{lower}") } else { lower }
}

/// Make `name` unique among `taken`, by appending a number.
fn unique(name: String, taken: &mut HashSet<String>) -> String {
    let mut n = name.clone();
    let mut i = 2;
    while !taken.insert(n.clone()) {
        n = format!("{name}{i}");
        i += 1;
    }
    n
}

struct Cs {
    /// The C# type without nullability, and whether it is a value type.
    ty: String,
    value_type: bool,
    /// Expression turning `{v}` (a value of `ty`) into what is bound.
    enum_name: Option<String>,
}

fn cs_type(t: &TypeIR, enums: &BTreeMap<String, String>) -> Cs {
    let (ty, value_type): (String, bool) = match t {
        TypeIR::Builtin(b) => match b {
            Builtin::Text | Builtin::Varchar(_) | Builtin::Char(_) | Builtin::Json => ("string".into(), false),
            Builtin::SmallInt => ("short".into(), true),
            Builtin::Int => ("int".into(), true),
            Builtin::BigInt => ("long".into(), true),
            Builtin::Decimal | Builtin::Numeric(..) => ("decimal".into(), true),
            Builtin::Real => ("float".into(), true),
            Builtin::Float => ("double".into(), true),
            Builtin::Bool => ("bool".into(), true),
            Builtin::Uuid => ("Guid".into(), true),
            Builtin::Timestamp => ("DateTimeOffset".into(), true),
            Builtin::TimestampNaive => ("DateTime".into(), true),
            Builtin::Date => ("DateOnly".into(), true),
            Builtin::Bytes => ("byte[]".into(), false),
        },
        TypeIR::Enum(n) => (enums.get(n).cloned().unwrap_or_else(|| pascal(n)), true),
        // composite types cannot appear in a parameter or result column
        TypeIR::Composite(_) => ("object".into(), false),
    };
    let enum_name = match t {
        TypeIR::Enum(n) => Some(n.clone()),
        _ => None,
    };
    Cs { ty, value_type, enum_name }
}

fn nullable_ty(c: &Cs, nullable: bool) -> String {
    if nullable { format!("{}?", c.ty) } else { c.ty.clone() }
}

/// Every enum the statements mention, by schema name.
fn used_enums(statements: &[Statement]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |t: &TypeIR| {
        if let TypeIR::Enum(n) = t
            && !out.contains(n)
        {
            out.push(n.clone());
        }
    };
    for s in statements {
        s.params().iter().for_each(|p| add(&p.ty));
        s.columns().iter().for_each(|c| add(&c.ty));
    }
    out.sort();
    out
}

fn verbatim(sql: &str) -> String { format!("@\"{}\"", sql.replace('"', "\"\"")) }

/// Generate one C# source file for `statements` (compiled against `schema`).
pub fn generate_csharp(schema: &SchemaIR, statements: &[Statement], opts: &CSharpOptions) -> String {
    let mut o = String::new();
    let w = &mut o;
    let _ = writeln!(w, "// <auto-generated>\n// Generated by `certo ql codegen`. Changes are lost when it runs again.\n// </auto-generated>");
    let _ = writeln!(w, "#nullable enable\n");
    let _ = writeln!(w, "using System;\nusing System.Collections.Generic;\nusing System.Data.Common;\nusing System.Threading;\nusing System.Threading.Tasks;\n");
    let _ = writeln!(w, "namespace {};\n", opts.namespace);

    // ---- enums ---------------------------------------------------------------------------
    let mut enum_names: BTreeMap<String, String> = BTreeMap::new();
    let mut taken_types: HashSet<String> = HashSet::new();
    taken_types.insert(opts.class_name.clone());
    for name in used_enums(statements) {
        let t = unique(pascal(&name), &mut taken_types);
        enum_names.insert(name, t);
    }
    for (db_name, cs_name) in &enum_names {
        let variants: Vec<&String> =
            schema.enums.iter().find(|e| &e.name == db_name).map(|e| e.variants.iter().collect()).unwrap_or_default();
        let mut used = HashSet::new();
        let names: Vec<String> = variants.iter().map(|v| unique(pascal(v), &mut used)).collect();
        let _ = writeln!(w, "public enum {cs_name}\n{{");
        for n in &names {
            let _ = writeln!(w, "    {n},");
        }
        let _ = writeln!(w, "}}\n");
        let _ = writeln!(w, "/// <summary>The database text of each {cs_name} value.</summary>\npublic static class {cs_name}Text\n{{");
        let _ = writeln!(w, "    public static string ToDb({cs_name} value) => value switch\n    {{");
        for (n, v) in names.iter().zip(&variants) {
            let _ = writeln!(w, "        {cs_name}.{n} => {},", verbatim(v));
        }
        let _ = writeln!(w, "        _ => throw new ArgumentOutOfRangeException(nameof(value)),\n    }};\n");
        let _ = writeln!(w, "    public static {cs_name} FromDb(string text) => text switch\n    {{");
        for (n, v) in names.iter().zip(&variants) {
            let _ = writeln!(w, "        {} => {cs_name}.{n},", verbatim(v));
        }
        let _ = writeln!(w, "        _ => throw new InvalidOperationException($\"unexpected {cs_name} value '{{text}}'\"),\n    }};\n}}\n");
    }

    // ---- rows ------------------------------------------------------------------------------
    // names first, so two statements that differ only in case or punctuation do not collide
    struct Names {
        base: String,
        row: String,
    }
    let mut names: Vec<Names> = Vec::new();
    for s in statements {
        let base = unique(pascal(s.name()), &mut taken_types);
        let row = format!("{base}Row");
        taken_types.insert(row.clone());
        names.push(Names { base, row });
    }
    for (s, n) in statements.iter().zip(&names) {
        if s.columns().is_empty() {
            continue;
        }
        let mut taken = HashSet::new();
        let fields: Vec<String> = s
            .columns()
            .iter()
            .map(|c| {
                let cs = cs_type(&c.ty, &enum_names);
                format!("{} {}", nullable_ty(&cs, c.nullable), unique(pascal(&c.name), &mut taken))
            })
            .collect();
        let _ = writeln!(w, "/// <summary>One row of <c>{}</c>.</summary>\npublic sealed record {}({});\n", s.name(), n.row, fields.join(", "));
    }

    // ---- statements --------------------------------------------------------------------------
    let _ = writeln!(w, "public static partial class {}\n{{", opts.class_name);
    for (s, n) in statements.iter().zip(&names) {
        let kind = match s {
            Statement::Query(_) => "query",
            Statement::Mutation(m) => match m.ir.kind {
                MutationKind::Insert => "insert",
                MutationKind::Update => "update",
                MutationKind::Delete => "delete",
            },
        };
        let has_rows = !s.columns().is_empty();
        let _ = writeln!(w, "    /// <summary>The SQL of <c>{} {}</c>.</summary>", kind, s.name());
        // PostgreSQL enum result columns are selected as text: a driver that does not know the
        // enum type (Npgsql without a mapping) cannot read them otherwise
        let opts_sql = LowerOptions { enums_as_text: true };
        let sql = match s {
            Statement::Query(q) => lower_with(opts.dialect, &q.ir, opts_sql).sql,
            Statement::Mutation(m) => lower_mutation_with(opts.dialect, &m.ir, opts_sql).sql,
        };
        let _ = writeln!(w, "    public const string {}Sql = {};\n", n.base, verbatim(&sql));

        // parameters, in declaration order
        let mut taken: HashSet<String> = ["connection", "transaction", "cancellationToken"].iter().map(|x| x.to_string()).collect();
        let params: Vec<(String, &crate::ParamIR, Cs)> = s
            .params()
            .iter()
            .map(|p| {
                let raw = camel(&p.name);
                let name = unique(raw.trim_start_matches('@').to_string(), &mut taken);
                let name = if KEYWORDS.contains(&name.as_str()) { format!("@{name}") } else { name };
                (name, p, cs_type(&p.ty, &enum_names))
            })
            .collect();
        let mut sig = vec!["this DbConnection connection".to_string()];
        for (name, p, cs) in &params {
            sig.push(format!("{} {name}", nullable_ty(cs, p.nullable)));
        }
        sig.push("DbTransaction? transaction = null".into());
        sig.push("CancellationToken cancellationToken = default".into());

        let ret = if has_rows { format!("Task<List<{}>>", n.row) } else { "Task<int>".to_string() };
        let _ = writeln!(w, "    /// <summary>Runs <c>{} {}</c>{}.</summary>", kind, s.name(),
            if has_rows || kind == "query" { "" } else { "; returns the number of rows affected" });
        let _ = writeln!(w, "    public static async {ret} {}Async({})\n    {{", n.base, sig.join(", "));
        let _ = writeln!(w, "        await using var command = connection.CreateCommand();");
        let _ = writeln!(w, "        command.CommandText = {}Sql;\n        command.Transaction = transaction;", n.base);
        for (i, placeholder) in s.param_order().iter().enumerate() {
            let Some((name, p, cs)) = params.iter().find(|(_, p, _)| &p.name == placeholder) else { continue };
            let value = bound_value(name, cs, p.nullable);
            let pname = match opts.dialect {
                Dialect::Sqlite => format!("\n            p.ParameterName = \"?{}\";", i + 1),
                Dialect::Postgres => String::new(), // Npgsql binds unnamed parameters by position ($1, $2, ...)
            };
            let _ = writeln!(w, "        {{\n            var p = command.CreateParameter();{pname}\n            p.Value = {value};\n            command.Parameters.Add(p);\n        }}");
        }
        if has_rows {
            let _ = writeln!(w, "        var rows = new List<{}>();", n.row);
            let _ = writeln!(w, "        await using var reader = await command.ExecuteReaderAsync(cancellationToken);");
            let _ = writeln!(w, "        while (await reader.ReadAsync(cancellationToken))\n        {{");
            let reads: Vec<String> = s
                .columns()
                .iter()
                .enumerate()
                .map(|(i, c)| read_value(i, &cs_type(&c.ty, &enum_names), c.nullable))
                .collect();
            let _ = writeln!(w, "            rows.Add(new {}(\n                {}));", n.row, reads.join(",\n                "));
            let _ = writeln!(w, "        }}\n        return rows;");
        } else {
            let _ = writeln!(w, "        return await command.ExecuteNonQueryAsync(cancellationToken);");
        }
        let _ = writeln!(w, "    }}\n");
    }
    let _ = writeln!(w, "}}");
    o
}

/// The C# expression bound as a parameter's value.
fn bound_value(name: &str, cs: &Cs, nullable: bool) -> String {
    let plain = name.trim_start_matches('@');
    match (&cs.enum_name, nullable) {
        (Some(_), false) => format!("{}Text.ToDb({name})", cs.ty),
        (Some(_), true) => format!("{name} is {{ }} {plain}Value ? {}Text.ToDb({plain}Value) : DBNull.Value", cs.ty),
        (None, false) => format!("(object){name}"),
        (None, true) => format!("(object?){name} ?? DBNull.Value"),
    }
}

/// The C# expression reading column `i` of the current row.
fn read_value(i: usize, cs: &Cs, nullable: bool) -> String {
    let get = match &cs.enum_name {
        Some(_) => format!("{}Text.FromDb(reader.GetString({i}))", cs.ty),
        None if cs.ty == "string" => format!("reader.GetString({i})"),
        None => format!("reader.GetFieldValue<{}>({i})", cs.ty),
    };
    if !nullable {
        return get;
    }
    if cs.value_type {
        format!("reader.IsDBNull({i}) ? ({}?)null : {get}", cs.ty)
    } else {
        format!("reader.IsDBNull({i}) ? null : {get}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile;

    const SCHEMA: &str = "enum Role { admin, user_ }
        table users { id: serial primary key  name: text not null  role: Role  born: date  balance: decimal(10,2) }
        table orders { id: serial primary key  user_id: int not null references users  total: decimal(10,2) not null }";

    fn gen_for(ql: &str, dialect: Dialect) -> String {
        let (ir, d) = certo_sdl::compile(SCHEMA);
        let schema = ir.unwrap_or_else(|| panic!("{d:?}"));
        let (s, d) = compile(&schema, ql, dialect);
        let s = s.unwrap_or_else(|| panic!("{d:?}"));
        generate_csharp(&schema, &s, &CSharpOptions { dialect, ..Default::default() })
    }

    #[test]
    fn a_query_becomes_a_record_and_an_extension_method() {
        let code = gen_for(
            "query recent_orders(min_total: decimal(10,2), since: date null) {
                from orders o join users u on o.user_id == u.id
                where o.total >= :min_total select o.id, u.name as customer, u.born, o.total }",
            Dialect::Postgres,
        );
        assert!(code.contains("public sealed record RecentOrdersRow(int Id, string Customer, DateOnly? Born, decimal Total);"), "{code}");
        assert!(
            code.contains("public static async Task<List<RecentOrdersRow>> RecentOrdersAsync(this DbConnection connection, decimal minTotal, DateOnly? since, DbTransaction? transaction = null, CancellationToken cancellationToken = default)"),
            "{code}"
        );
        assert!(code.contains("public const string RecentOrdersSql = @\"SELECT"), "{code}");
        // `since` is declared but never used, so only minTotal is bound
        assert!(code.contains("p.Value = (object)minTotal;") && !code.contains("since ??"), "{code}");
        assert!(code.contains("reader.IsDBNull(2) ? (DateOnly?)null : reader.GetFieldValue<DateOnly>(2)"), "{code}");
        assert!(code.contains("reader.GetString(1)") && code.contains("reader.GetFieldValue<decimal>(3)"), "{code}");
        // PostgreSQL binds unnamed parameters by position; SQLite names them
        assert!(!code.contains("ParameterName"));
        assert!(gen_for("query q(n: int) { from users u where u.id == :n select u.id }", Dialect::Sqlite).contains("p.ParameterName = \"?1\";"));
    }

    #[test]
    fn enums_become_csharp_enums_with_their_database_text() {
        let code = gen_for(
            "query by_role(r: Role null) { from users u where :r is null or u.role == :r select u.id, u.role }
             insert add(n: text, r: Role) { into users set name = :n, role = :r returning role }",
            Dialect::Postgres,
        );
        assert!(code.contains("public enum Role\n{\n    Admin,\n    User,\n}"), "{code}");
        assert!(code.contains("\"user_\" => Role.User,") && code.contains("Role.Admin => @\"admin\","), "{code}");
        assert!(code.contains("public sealed record ByRoleRow(int Id, Role? Role);"), "{code}");
        assert!(code.contains("r is { } rValue ? RoleText.ToDb(rValue) : DBNull.Value"), "{code}");
        assert!(code.contains("p.Value = RoleText.ToDb(r);"), "{code}");
        assert!(code.contains("RoleText.FromDb(reader.GetString(1))"), "{code}");
        // PostgreSQL selects enum results as text so drivers need no enum mapping; SQLite needs nothing
        assert!(code.contains("::text AS \"\"role\"\""), "{code}");
        let lite = gen_for("query q() { from users u select u.role }", Dialect::Sqlite);
        assert!(!lite.contains("::text"), "{lite}");
    }

    #[test]
    fn mutations_return_counts_or_rows() {
        let code = gen_for(
            "insert add(n: text) { into users set name = :n returning id }
             update rename(id: int, n: text) { users u set name = :n where u.id == :id }
             delete purge() { from orders o all rows }",
            Dialect::Postgres,
        );
        assert!(code.contains("Task<List<AddRow>> AddAsync(") && code.contains("public sealed record AddRow(int Id);"), "{code}");
        assert!(code.contains("Task<int> RenameAsync(") && code.contains("return await command.ExecuteNonQueryAsync(cancellationToken);"), "{code}");
        assert!(code.contains("Task<int> PurgeAsync(this DbConnection connection, DbTransaction? transaction"), "{code}");
        assert!(!code.contains("record RenameRow") && !code.contains("record PurgeRow"));
        // parameters are bound in placeholder order, not declaration order: `n` is first in the SQL
        let rename = code.split("RenameAsync").nth(1).unwrap();
        assert!(rename.find("p.Value = (object)n;").unwrap() < rename.find("p.Value = (object)id;").unwrap(), "{rename}");
    }

    #[test]
    fn names_are_made_safe() {
        let code = gen_for(
            "query class_list(default: int, string_val: text null) { from users u where u.id == :default and u.name == :string_val select u.id as in_ }
             query class_list2() { from users u select u.id }",
            Dialect::Postgres,
        );
        assert!(code.contains("int @default, string? stringVal"), "{code}");
        assert!(code.contains("ClassListRow(int In)"), "{code}");
        // two statements whose names collapse to the same C# name stay distinct
        let code = gen_for(
            "query a_b() { from users u select u.id } query aB() { from users u select u.id }",
            Dialect::Postgres,
        );
        assert!(code.contains("ABAsync(") && code.contains("AB2Async("), "{code}");
    }

    #[test]
    fn options_choose_the_namespace_and_class() {
        let (ir, _) = certo_sdl::compile(SCHEMA);
        let schema = ir.unwrap();
        let (s, _) = compile(&schema, "query q() { from users u select u.id }", Dialect::Postgres);
        let code = generate_csharp(
            &schema,
            &s.unwrap(),
            &CSharpOptions { namespace: "My.App".into(), class_name: "Db".into(), dialect: Dialect::Postgres },
        );
        assert!(code.contains("namespace My.App;") && code.contains("public static partial class Db"), "{code}");
        assert!(code.starts_with("// <auto-generated>"));
    }
}
