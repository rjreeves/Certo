use crate::span::{S, Span};
use crate::types::{TypeExpr, TypeParam, EffectSet, ModulePath, Ident};
use crate::expr::Expr;
use crate::pattern::Pattern;

// ------------------------------------------------------------------ //
// Top-level declarations
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    /// `fn name<T>(params): RetType [effects] = body`
    Fn(FnDecl),

    /// `type Name<T> = ...`
    Type(TypeDecl),

    /// `val name: Type = expr`  (module-level immutable binding)
    Val(ValDecl),

    /// `var name: Type = expr`  (module-level mutable binding)
    Var(VarDecl),

    /// `trait Name<T> { ... }`
    Trait(TraitDecl),

    /// `impl Trait for Type { ... }`
    Impl(ImplDecl),

    /// `statemachine Name { ... }`
    StateMachine(StateMachineDecl),

    /// `pub? validator Name for Type errors Type { ... }`
    Validator(ValidatorDecl),

    /// `pub? constraint Name = expr`
    Constraint(ConstraintDecl),

    /// `pub? temporal Name = expr`
    Temporal(TemporalDecl),

    /// `ruleTest Validator.rule "label" { ... }`
    RuleTest(RuleTestDecl),

    /// `validatorTest Validator "label" { ... }`
    ValidatorTest(ValidatorTestDecl),

    /// `migration "name" { up { ... } down { ... } }`
    Migration(MigrationDecl),

    /// `view Name { ... }`
    View(ViewDecl),

    /// `form Name { ... }`
    Form(FormDecl),

    /// `@ui.generate(TypeName) { title: "...", list: { columns: [...] } }`
    UiGenerate(UiGenerateDecl),

    /// `test "name" { ... }`
    Test(TestDecl),

    /// `property "name" { ... }`
    Property(PropertyDecl),

    /// `dbTest "name" { ... }`
    DbTest(DbTestDecl),

    /// `import Stdlib.Text` — import a stdlib module
    Import(ImportDecl),
}

// ------------------------------------------------------------------ //
// Function declaration
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    pub is_async:    bool,
    pub is_pub:      bool,
    pub name:        Ident,
    pub type_params: Vec<TypeParam>,
    pub params:      Vec<FnParam>,
    pub ret_ty:      Option<S<TypeExpr>>,
    pub effects:     Option<EffectSet>,
    pub body:        Option<S<Expr>>,   // None for trait method signatures and `extern` declarations
    pub is_extern:   bool,              // true for fns declared in an `extern "C"` block (no body; linked externally)
    /// `@export("name")` — overrides the generated C export symbol name
    /// (default `certo_<name>`). Only valid on `pub fn`; enforced by the parser.
    pub export_name: Option<String>,
    pub span:        Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnParam {
    pub name:    Ident,
    pub ty:      S<TypeExpr>,
    pub default: Option<S<Expr>>,
    pub span:    Span,
}

// ------------------------------------------------------------------ //
// Type declaration
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    pub is_pub:      bool,
    /// `type X = priv X(...)` — a single-constructor "smart constructor"
    /// newtype whose raw constructor may only be called from within an
    /// `impl X { ... }` block for the same type (typically a validating
    /// `X.new` factory); everywhere else must go through that factory.
    /// Only meaningful when `body` is `TypeBody::Sum` with exactly one
    /// variant named the same as `name`.
    pub is_priv_ctor: bool,
    pub name:        Ident,
    pub type_params: Vec<TypeParam>,
    pub body:        TypeBody,
    pub span:        Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeBody {
    /// `= { field: Type, ... }` — product type / record
    Record(RecordTypeDef),

    /// `= | Variant(...) | Variant(...)` — sum type / enum
    Sum(Vec<SumVariant>),

    /// `= OtherType` — type alias
    Alias(S<TypeExpr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordTypeDef {
    pub fields:   Vec<RecordFieldDef>,
    pub computed: Vec<ComputedFieldDef>,
    pub span:     Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordFieldDef {
    pub name:     Ident,
    pub ty:       S<TypeExpr>,
    pub optional: bool,
    pub span:     Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedFieldDef {
    pub name:  Ident,
    pub ty:    S<TypeExpr>,
    pub body:  S<Expr>,
    pub span:  Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SumVariant {
    pub name:   Ident,
    /// Named fields: `Circle(radius: Float)` — empty for unit variants
    pub fields: Vec<VariantField>,
    pub span:   Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantField {
    pub name: Option<Ident>,  // None for positional fields
    pub ty:   S<TypeExpr>,
    pub span: Span,
}

// ------------------------------------------------------------------ //
// Value / variable declarations
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct ValDecl {
    pub is_pub:  bool,
    pub pattern: S<Pattern>,
    pub ty:      Option<S<TypeExpr>>,
    pub value:   S<Expr>,
    pub span:    Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub is_pub: bool,
    pub name:   Ident,
    pub ty:     Option<S<TypeExpr>>,
    pub value:  S<Expr>,
    pub span:   Span,
}

// ------------------------------------------------------------------ //
// Trait and impl declarations
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct TraitDecl {
    pub is_pub:      bool,
    pub name:        Ident,
    pub type_params: Vec<TypeParam>,
    pub methods:     Vec<FnDecl>,
    pub span:        Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImplDecl {
    pub trait_path:  Option<ModulePath>,  // None for inherent impls
    pub type_path:   ModulePath,
    pub type_params: Vec<TypeParam>,
    pub methods:     Vec<FnDecl>,
    pub span:        Span,
}

// ------------------------------------------------------------------ //
// State machine declaration
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct StateMachineDecl {
    pub name:        Ident,
    pub states:      Vec<Ident>,
    pub transitions: Vec<Transition>,
    pub on_enter:    Vec<OnEnterHook>,
    pub invariants:  Vec<Invariant>,
    pub span:        Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub from:   Ident,
    pub to:     Ident,
    pub event:  Ident,
    pub params: Vec<FnParam>,
    pub span:   Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OnEnterHook {
    pub state: Ident,
    pub body:  S<Expr>,
    pub span:  Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Invariant {
    pub state: Ident,
    pub cond:  S<Expr>,
    pub span:  Span,
}

// ------------------------------------------------------------------ //
// Constraint declaration
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct ConstraintDecl {
    pub is_pub: bool,
    pub name:   Ident,
    pub body:   S<Expr>,   // boolean expression — type checked lazily at use site
    pub span:   Span,
}

// ------------------------------------------------------------------ //
// Temporal declaration
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct TemporalDecl {
    pub is_pub: bool,
    pub name:   Ident,
    pub body:   S<Expr>,   // must resolve to Duration — verified in type checker
    pub span:   Span,
}

// ------------------------------------------------------------------ //
// Validator declaration
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatorDecl {
    pub is_pub:  bool,
    pub name:    Ident,
    pub entity:  S<TypeExpr>,            // the `for` type
    pub errors:  S<TypeExpr>,            // the `errors` type
    pub trigger: Option<TriggerDecl>,
    pub context: Vec<ContextField>,
    pub rules:   Vec<RuleDecl>,
    pub span:    Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerDecl {
    pub op:        TriggerOp,
    pub condition: Option<TriggerCondition>,
    pub span:      Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerOp { Insert, Update }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerCondOp { Eq, NotEq }

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerCondition {
    pub field: Ident,
    pub op:    TriggerCondOp,
    pub value: S<Expr>,
    pub span:  Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContextField {
    pub name:      Ident,
    pub type_ref:  S<TypeExpr>,
    pub loaded_by: Option<S<Expr>>,
    pub span:      Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuleDecl {
    pub name:      Ident,
    pub after:     Vec<Ident>,        // prerequisite rule names
    pub overrides: Option<Ident>,     // rule this suspends
    pub priority:  Option<i64>,
    pub require:   S<Expr>,           // boolean condition
    pub else_:     S<Expr>,           // produces errors type
    pub span:      Span,
}

// ------------------------------------------------------------------ //
// Test declarations
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct RuleTestDecl {
    pub validator: Vec<Ident>,   // qualified path: [ValidatorName, rule_name]
    pub label:     String,
    pub entity:    S<Expr>,
    pub context:   S<Expr>,
    pub expect:    TestExpectation,
    pub span:      Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatorTestDecl {
    pub validator: Ident,
    pub label:     String,
    pub entity:    S<Expr>,
    pub context:   S<Expr>,
    pub expect:    TestExpectation,
    pub span:      Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TestExpectation {
    Pass,
    Fail { with: Option<S<Expr>> },
}

// ------------------------------------------------------------------ //
// Migration declaration
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct MigrationDecl {
    pub name:        String,
    pub description: Option<String>,
    pub up:          Vec<MigrationOp>,
    pub down:        Vec<MigrationOp>,
    pub span:        Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MigrationOp {
    CreateTable   { name: String, columns: Vec<ColumnDef>, span: Span },
    AlterTable    { name: String, ops: Vec<AlterOp>,       span: Span },
    DropTable     { name: String,                           span: Span },
    CreateIndex   { name: String, table: String, columns: Vec<String>, span: Span },
    DropIndex     { name: String,                           span: Span },
    RawSql        { sql: String,                            span: Span },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub name:        String,
    pub ty:          S<TypeExpr>,
    pub primary_key: bool,
    pub nullable:    bool,
    pub unique:      bool,
    pub default:     Option<S<Expr>>,
    pub span:        Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AlterOp {
    AddColumn    { def: ColumnDef },
    DropColumn   { name: String, span: Span },
    AddForeignKey{ column: String, references: String, on_delete: FkAction, span: Span },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FkAction { Cascade, SetNull, Restrict, NoAction }

// ------------------------------------------------------------------ //
// UI declarations (view / form)
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct ViewDecl {
    pub name:      Ident,
    pub live:      Vec<ValDecl>,
    pub layout:    S<Expr>,
    pub pk:        Option<String>,   // primary key column (camelCase field name)
    pub filter_by: Option<String>,   // FK column to filter on (?field=X in URL)
    pub span:      Span,
}

/// `@ui.generate(TypeName) { title: "...", list: { columns: [...] } }` —
/// BACKLOG item 87, deliberately scoped to a single list view + create/edit
/// forms lowered from the annotated type's own fields. The spec's fuller
/// example (`sortable`/`filterable`/`searchable` columns, a `detail:`
/// section view, separate `form.create`/`form.edit` field sets, and a
/// `permissions:` block) each need real, currently-nonexistent capability
/// in `crates/ui` — not attempted here, not silently accepted either (the
/// parser rejects any key besides `title`/`list.columns`).
#[derive(Debug, Clone, PartialEq)]
pub struct UiGenerateDecl {
    pub type_name: Ident,
    pub title:     Option<String>,
    pub columns:   Vec<String>,
    pub span:      Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormDecl {
    pub name:       Ident,
    pub target:     ModulePath,
    pub fields:     Vec<FormField>,
    pub pk:         Option<String>,  // if set, generates UPDATE WHERE pk=$N instead of INSERT
    pub on_submit:  Option<S<Expr>>,
    pub on_success: Option<S<Expr>>,
    pub span:       Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormField {
    pub name:        Ident,
    pub label:       Option<String>,
    pub placeholder: Option<String>,
    pub field_type:  Option<S<Expr>>,
    pub options:     Option<S<Expr>>,
    pub rows:        Option<u32>,
    pub span:        Span,
}

// ------------------------------------------------------------------ //
// Test declarations
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct TestDecl {
    pub name: String,
    pub body: S<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PropertyDecl {
    pub name:   String,
    /// Typed inputs the test runner generates random values for and shrinks
    /// on failure (BACKLOG item 86). Empty for a plain `property "name" { .. }`
    /// with no declared inputs, which still just runs once — same as before
    /// this field existed.
    pub params: Vec<FnParam>,
    pub body:   S<Expr>,
    pub span:   Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DbTestDecl {
    pub name: String,
    pub body: S<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportDecl {
    /// The full import path, e.g. `["Stdlib", "Text"]`
    pub path: Vec<String>,
    pub span: Span,
}
