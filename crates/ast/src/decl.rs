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

    /// `validator Name { ... }`
    Validator(ValidatorDecl),

    /// `migration "name" { up { ... } down { ... } }`
    Migration(MigrationDecl),

    /// `view Name { ... }`
    View(ViewDecl),

    /// `form Name { ... }`
    Form(FormDecl),

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
    pub body:        Option<S<Expr>>,   // None for trait method signatures
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
// Validator declaration
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatorDecl {
    pub name:  Ident,
    pub rules: Vec<ValidatorRule>,
    pub span:  Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatorRule {
    pub field: Ident,
    pub expr:  S<Expr>,
    pub span:  Span,
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
    pub name: String,
    pub body: S<Expr>,
    pub span: Span,
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
