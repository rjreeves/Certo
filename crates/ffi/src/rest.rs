//! Generate typed Certo client stubs from a JSON REST API schema (and, via
//! `openapi.rs`, from an OpenAPI 3.x document — both funnel into the
//! `RestSchema` this module defines and share this module's codegen).
//!
//! # Schema format
//!
//! ```json
//! {
//!   "module":   "UserApi",
//!   "base_url": "https://api.example.com",
//!   "models": {
//!     "User": [
//!       { "name": "id", "ty": "Int" },
//!       { "name": "name", "ty": "Text" },
//!       { "name": "email", "ty": "Text", "optional": true }
//!     ],
//!     "CreateUserRequest": [
//!       { "name": "name", "ty": "Text" }
//!     ]
//!   },
//!   "endpoints": [
//!     {
//!       "name":         "getUser",
//!       "method":       "GET",
//!       "path":         "/users/{id}",
//!       "path_params":  [{ "name": "id", "ty": "Int" }],
//!       "response":     "User"
//!     },
//!     {
//!       "name":     "createUser",
//!       "method":   "POST",
//!       "path":     "/users",
//!       "body":     { "ty": "CreateUserRequest" },
//!       "response": "User"
//!     }
//!   ]
//! }
//! ```
//!
//! # Generated Certo (abbreviated — see `emit_model`/`emit_endpoint`)
//!
//! ```certo
//! module UserApi
//!
//! type User = { id: Int, name: Text, email: Text? }
//!
//! fn __decodeUser(jv: JsonValue): User = ...
//! fn __encodeUser(v: User): JsonValue = ...
//!
//! // GET /users/{id}
//! pub fn getUser(id: Int): Result<User, Text> [io] = {
//!     val resp = Http.get("https://api.example.com/users/" ++ intToText(id))
//!     if HttpResponse.ok(resp)
//!         then Ok(__decodeUser(Json.parse(HttpResponse.body(resp))))
//!         else Err(HttpResponse.body(resp))
//! }
//! ```
//!
//! # Scope (deliberate, not a design-unknown — see BACKLOG item 93)
//!
//! A model field's type may be a primitive (`Int`/`Float`/`Bool`/`Text`),
//! optional (`T?`), or a `List<T>` of those (recursively) — including
//! another model by name. Two things are deliberately **not** attempted:
//! an inline/anonymous object shape that doesn't correspond to a named
//! model (`oneOf`/`anyOf`/free-form JSON) is rejected with a clear error
//! naming the offending field rather than silently emitting `void*`/`Text`
//! junk; and per-status-code typed error variants (the spec's own
//! `Result<Charge, StripeError>` example) are not generated — every
//! endpoint's error case is `Result<T, Text>` (the raw response body on a
//! non-2xx status), a real but simpler mechanism, since deriving a whole
//! sum type per documented error response is materially more design work
//! than reusing the model/decode machinery this item already needs.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Deserialize;

use crate::error::FfiError;

// ------------------------------------------------------------------ //
// Schema types
// ------------------------------------------------------------------ //

#[derive(Debug, Deserialize)]
pub struct RestSchema {
    /// Certo module name for the generated client file.
    pub module:    String,
    /// Base URL prepended to all endpoint paths.
    pub base_url:  String,
    /// Named record shapes referenced by endpoints' `response`/`body.ty`.
    /// Insertion order is preserved (`BTreeMap` here would resort
    /// alphabetically — an `IndexMap` isn't a dependency already, so this
    /// uses a plain `Vec` under a custom deserializer instead; see below).
    #[serde(default, deserialize_with = "deserialize_models")]
    pub models: Vec<Model>,
    pub endpoints: Vec<Endpoint>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Model {
    #[serde(skip)]
    pub name: String,
    pub fields: Vec<ModelField>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelField {
    pub name:     String,
    /// A `FieldTy` string: `Int`/`Float`/`Bool`/`Text`, `List<...>`, or
    /// another model's name (resolved against `RestSchema.models`).
    pub ty:       String,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Deserialize)]
pub struct Endpoint {
    /// Certo function name.
    pub name:         String,
    /// HTTP method (`"GET"`, `"POST"`, `"PUT"`, `"PATCH"`, `"DELETE"`, ...).
    pub method:       String,
    /// URL path, with `{param}` placeholders for path parameters.
    pub path:         String,
    #[serde(default)]
    pub path_params:  Vec<Param>,
    #[serde(default)]
    pub query_params: Vec<Param>,
    /// Request body type. `null` / absent for GET/DELETE.
    pub body:         Option<BodyDef>,
    /// Response type name: a primitive, a model name, `List<...>` of
    /// either, or `null`/absent for `Unit` (no body expected).
    pub response:     Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Param {
    pub name:     String,
    pub ty:       String,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Deserialize)]
pub struct BodyDef {
    pub ty: String,
    /// Defaults to `application/json` — the only encoding this generator
    /// actually knows how to serialize a model into.
    #[serde(default = "default_content_type")]
    pub content_type: String,
}

fn default_content_type() -> String { "application/json".to_string() }

/// `models` is a JSON *object* (`{ "User": { "fields": [...] } }`), but
/// `Vec<Model>` needs each entry's key threaded into `Model.name` (the
/// straightforward `#[serde] HashMap<String, Model>` loses both the name
/// and the declaration order the generated Certo file should preserve).
fn deserialize_models<'de, D>(deserializer: D) -> Result<Vec<Model>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let map: BTreeMap<String, RawModel> = BTreeMap::deserialize(deserializer)?;
    Ok(map.into_iter().map(|(name, raw)| Model { name, fields: raw.fields }).collect())
}

#[derive(Debug, Deserialize)]
struct RawModel {
    fields: Vec<ModelField>,
}

// ------------------------------------------------------------------ //
// Field type resolution — shared by codegen for both models and
// endpoint response/body types, since a `List<Model>` response is
// structurally the same decode/encode problem as a `List<T>` field.
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
enum FieldTy {
    Int,
    Float,
    Bool,
    Text,
    Unit,
    List(Box<FieldTy>),
    Model(String),
}

impl FieldTy {
    /// Parse a schema type string. `models` is checked last, after every
    /// primitive/List spelling, so a model can't accidentally shadow a
    /// builtin name.
    fn parse(raw: &str, models: &[Model]) -> Result<FieldTy, String> {
        let raw = raw.trim();
        if let Some(inner) = raw.strip_prefix("List<").and_then(|s| s.strip_suffix('>')) {
            let inner_ty = FieldTy::parse(inner, models)?;
            // A model is a real multi-field C struct passed by value;
            // List<T>'s element slot is a pointer-sized void*, boxed via
            // the same convention crates/codegen/src/emit_mir.rs's
            // box_value uses everywhere — casting a struct through
            // intptr_t doesn't compile. Confirmed directly (not
            // theorized): the same pre-existing compiler gap BACKLOG item
            // 124 already found and scoped out for property-test
            // generation's List<Record> also breaks a generated
            // List<Model> response/field here, for the identical reason.
            // A List<List<...>> of a model has the same problem at
            // whichever depth the model appears, so this check applies
            // recursively (a List<T> is itself always pointer-sized
            // regardless of T, so nesting lists is fine — only a bare
            // model as the *immediate* element is rejected).
            if matches!(inner_ty, FieldTy::Model(_)) {
                return Err(format!(
                    "List<{inner}> isn't supported — a model is a multi-field struct passed by \
                     value, and List<T>'s element slot can't hold one without heap-boxing support \
                     the compiler doesn't have yet (BACKLOG item 120); List<T> works for Int/Float/\
                     Bool/Text/List<...> of those"
                ));
            }
            return Ok(FieldTy::List(Box::new(inner_ty)));
        }
        match raw {
            "Int"   => return Ok(FieldTy::Int),
            "Float" => return Ok(FieldTy::Float),
            "Bool"  => return Ok(FieldTy::Bool),
            "Text"  => return Ok(FieldTy::Text),
            "Unit"  => return Ok(FieldTy::Unit),
            _ => {}
        }
        if models.iter().any(|m| m.name == raw) {
            return Ok(FieldTy::Model(raw.to_string()));
        }
        Err(format!(
            "unknown type `{raw}` — expected Int, Float, Bool, Text, Unit, List<T>, or a name declared in `models`"
        ))
    }

    /// The Certo type annotation this decodes/encodes to (before any `?`
    /// an *optional* field/param layers on top — see `certo_ty`).
    fn certo_ty(&self) -> String {
        match self {
            FieldTy::Int   => "Int".to_string(),
            FieldTy::Float => "Float".to_string(),
            FieldTy::Bool  => "Bool".to_string(),
            FieldTy::Text  => "Text".to_string(),
            FieldTy::Unit  => "Unit".to_string(),
            FieldTy::List(inner) => format!("List<{}>", inner.certo_ty()),
            FieldTy::Model(name) => name.clone(),
        }
    }

    /// A JSON-encodable value (`Int`/`Float`/`Bool`/`Text`/a model whose
    /// own `__encodeX` exists) always decodes via a `JsonValue` accessor or
    /// a call — `Unit` never appears as a field/model type in practice
    /// (only as a whole endpoint's response), so it has no decode form.
    fn decode_expr(&self, jv_expr: &str) -> String {
        match self {
            FieldTy::Int   => format!("JsonValue.asInt({jv_expr})"),
            FieldTy::Float => format!("JsonValue.asFloat({jv_expr})"),
            FieldTy::Bool  => format!("JsonValue.asBool({jv_expr})"),
            FieldTy::Text  => format!("JsonValue.asText({jv_expr})"),
            FieldTy::Unit  => "{}".to_string(), // see the `None => {{}}` comment in emit_model re: Unit not being a real value literal
            FieldTy::Model(name) => format!("{}({jv_expr})", decoder_fn_name(name)),
            FieldTy::List(inner) => {
                // Materializes via a loop — can't be a pure expression the
                // way a scalar decode can, so this branch is only reached
                // through `emit_decode_list_block` (see below), never
                // inlined directly by a caller expecting one expression.
                unreachable!("List decode must go through emit_decode_list_block, inner={:?}", inner)
            }
        }
    }

    fn encode_expr(&self, val_expr: &str) -> String {
        match self {
            FieldTy::Int   => format!("Json.int({val_expr})"),
            FieldTy::Float => format!("Json.float({val_expr})"),
            FieldTy::Bool  => format!("Json.bool({val_expr})"),
            FieldTy::Text  => format!("Json.string({val_expr})"),
            FieldTy::Unit  => "Json.null()".to_string(),
            FieldTy::Model(name) => format!("{}({val_expr})", encoder_fn_name(name)),
            FieldTy::List(_) => unreachable!("List encode must go through emit_encode_list_block"),
        }
    }

    fn to_text_expr(&self, val_expr: &str) -> String {
        match self {
            FieldTy::Int   => format!("intToText({val_expr})"),
            FieldTy::Float => format!("floatToText({val_expr})"),
            FieldTy::Bool  => format!("boolToText({val_expr})"),
            FieldTy::Text  => val_expr.to_string(),
            other => format!("/* unsupported path/query param type {other:?} */ {val_expr}"),
        }
    }
}

fn decoder_fn_name(model: &str) -> String { format!("__decode{}", model) }
fn encoder_fn_name(model: &str) -> String { format!("__encode{}", model) }

// ------------------------------------------------------------------ //
// Public API
// ------------------------------------------------------------------ //

/// Parse a REST schema from a JSON string.
pub fn parse_schema(json: &str) -> Result<RestSchema, FfiError> {
    let schema: RestSchema = serde_json::from_str(json).map_err(|e| FfiError::SchemaError(e.to_string()))?;
    validate_schema(&schema)?;
    Ok(schema)
}

/// Every model field type and every endpoint's path/query/body/response
/// type must resolve — checked once up front so codegen never has to
/// handle an unresolved type, and the caller gets one clear error naming
/// exactly what's wrong instead of a confusing generated-code failure.
pub(crate) fn validate_schema(schema: &RestSchema) -> Result<(), FfiError> {
    for m in &schema.models {
        for f in &m.fields {
            FieldTy::parse(&f.ty, &schema.models)
                .map_err(|e| FfiError::SchemaError(format!("model `{}` field `{}`: {}", m.name, f.name, e)))?;
        }
    }
    for ep in &schema.endpoints {
        for p in ep.path_params.iter().chain(&ep.query_params) {
            FieldTy::parse(&p.ty, &schema.models)
                .map_err(|e| FfiError::SchemaError(format!("endpoint `{}` param `{}`: {}", ep.name, p.name, e)))?;
        }
        if let Some(body) = &ep.body {
            FieldTy::parse(&body.ty, &schema.models)
                .map_err(|e| FfiError::SchemaError(format!("endpoint `{}` body: {}", ep.name, e)))?;
        }
        if let Some(resp) = &ep.response {
            FieldTy::parse(resp, &schema.models)
                .map_err(|e| FfiError::SchemaError(format!("endpoint `{}` response: {}", ep.name, e)))?;
        }
    }
    Ok(())
}

/// Generate a Certo source file (module + model types/codecs + `pub fn`
/// stubs) from a validated `RestSchema`.
pub fn generate_client(schema: &RestSchema) -> String {
    let mut out = String::new();

    writeln!(out, "module {}", schema.module).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "// REST client generated by certo-ffi — do not edit").unwrap();
    writeln!(out, "// Base URL: {}", schema.base_url).unwrap();

    for m in &schema.models {
        writeln!(out).unwrap();
        emit_model(&mut out, m, &schema.models);
    }

    for ep in &schema.endpoints {
        writeln!(out).unwrap();
        emit_endpoint(&mut out, ep, &schema.base_url, &schema.models);
    }

    out
}

// ------------------------------------------------------------------ //
// Model codegen — type declaration + decode/encode functions
// ------------------------------------------------------------------ //

fn emit_model(out: &mut String, m: &Model, models: &[Model]) {
    let field_tys: Vec<(String, FieldTy, bool)> = m.fields.iter()
        .map(|f| (f.name.clone(), FieldTy::parse(&f.ty, models).expect("validated"), f.optional))
        .collect();

    // Type declaration.
    let fields_str = field_tys.iter()
        .map(|(name, ty, optional)| format!("{}: {}{}", name, ty.certo_ty(), if *optional { "?" } else { "" }))
        .collect::<Vec<_>>().join(", ");
    writeln!(out, "type {} = {{ {} }}", m.name, fields_str).unwrap();
    writeln!(out).unwrap();

    // Decoder: fn __decodeX(jv: JsonValue): X
    writeln!(out, "fn {}(jv: JsonValue): {} = {{", decoder_fn_name(&m.name), m.name).unwrap();
    let mut inits = Vec::new();
    for (name, ty, optional) in &field_tys {
        let field_jv = format!("JsonValue.get(jv, \"{}\")", name);
        if *optional {
            let local = format!("__jv_{name}");
            writeln!(out, "    val {local} = {field_jv}").unwrap();
            let decoded = emit_decode_value(out, ty, &local, &format!("_{name}"));
            writeln!(out, "    val {name} = if JsonValue.isNull({local}) then None else Some({decoded})").unwrap();
        } else {
            let decoded = emit_decode_value(out, ty, &field_jv, &format!("_{name}"));
            writeln!(out, "    val {name} = {decoded}").unwrap();
        }
        inits.push(name.clone());
    }
    let ctor_args = inits.iter().map(|n| format!("{n}: {n}")).collect::<Vec<_>>().join(", ");
    writeln!(out, "    {} {{ {} }}", m.name, ctor_args).unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Encoder: fn __encodeX(v: X): JsonValue
    writeln!(out, "fn {}(v: {}): JsonValue = {{", encoder_fn_name(&m.name), m.name).unwrap();
    writeln!(out, "    val obj = Json.object()").unwrap();
    for (name, ty, optional) in &field_tys {
        let field_val = format!("v.{}", name);
        if *optional {
            // A composite field type (List<T>) needs real statements to
            // encode (see emit_encode_value) — those must land *inside*
            // the `Some(x) => { ... }` arm's own block, not before the
            // `match` itself (which is what happens if you naively pass
            // the outer `out` buffer straight through here: the loop runs
            // unconditionally for a field that might be None, and worse,
            // the emitted statements end up as invalid syntax sitting
            // between `match { ` and its first arm). Confirmed as a real,
            // reproducible compile error — not theorized — while
            // generating a client from an OpenAPI spec whose `tags` field
            // wasn't in `required` (making it Text? for a scalar field,
            // but List<Text>? here), which this item's original
            // hand-written test schema's non-optional `tags` field never
            // exercised. Route through a separate buffer first so the
            // decision of "one-line arm" vs "block arm" can be made based
            // on whether any statements actually got produced.
            let mut arm_body = String::new();
            let inner_expr = emit_encode_value(&mut arm_body, ty, "x", &format!("_{name}"), "            ");
            writeln!(out, "    match {field_val} {{").unwrap();
            if arm_body.is_empty() {
                writeln!(out, "        Some(x) => JsonValue.set(obj, \"{name}\", {inner_expr})").unwrap();
            } else {
                writeln!(out, "        Some(x) => {{").unwrap();
                out.push_str(&arm_body);
                writeln!(out, "            JsonValue.set(obj, \"{name}\", {inner_expr})").unwrap();
                writeln!(out, "        }}").unwrap();
            }
            // `Unit` is not a real value literal in Certo (confirmed
            // against typeck: "undefined name `Unit`" — the GUIDE.md
            // example that suggests it is itself stale/wrong). An empty
            // block `{}` genuinely infers as `Ty::Unit` by default
            // (`infer_block`'s `last_ty` starts at `Ty::Unit` and only
            // changes if there's at least one statement), so that's the
            // real way to write a bare Unit value.
            writeln!(out, "        None => {{}}").unwrap();
            writeln!(out, "    }}").unwrap();
        } else {
            let encoded = emit_encode_value(out, ty, &field_val, &format!("_{name}"), "    ");
            writeln!(out, "    JsonValue.set(obj, \"{name}\", {encoded})").unwrap();
        }
    }
    writeln!(out, "    obj").unwrap();
    writeln!(out, "}}").unwrap();
}

/// Decode a single (non-optional) value of `ty` from the `JsonValue`
/// expression `jv_expr`. Scalars and models are pure expressions; a
/// `List<T>` needs a loop, so it's materialized into a fresh `var` (named
/// from `hint`) via statements written to `out`, returning that variable's
/// name instead of an inline expression.
fn emit_decode_value(out: &mut String, ty: &FieldTy, jv_expr: &str, hint: &str) -> String {
    match ty {
        FieldTy::List(inner) => {
            // Bind the source JsonValue once — `jv_expr` can be an
            // arbitrarily expensive expression (a whole-response decode
            // passes `Json.parse(HttpResponse.body(__resp))` here), and
            // it's referenced twice below (JsonValue.length, then again
            // every loop iteration for JsonValue.at); inlining it directly
            // would re-parse the response body once per element.
            let src = format!("__src{hint}");
            let acc = format!("__acc{hint}");
            let i   = format!("__i{hint}");
            writeln!(out, "    val {src} = {jv_expr}").unwrap();
            writeln!(out, "    var {acc} = List.empty()").unwrap();
            writeln!(out, "    for {i} in range(0, JsonValue.length({src})) {{").unwrap();
            let elem_jv = format!("JsonValue.at({src}, {i})");
            let elem_expr = match inner.as_ref() {
                FieldTy::List(_) => {
                    // Nested list — recurse into its own statement block first.
                    emit_decode_value(out, inner, &elem_jv, &format!("{hint}_e"))
                }
                other => other.decode_expr(&elem_jv),
            };
            writeln!(out, "        {acc} = List.push({acc}, {elem_expr})").unwrap();
            writeln!(out, "    }}").unwrap();
            acc
        }
        other => other.decode_expr(jv_expr),
    }
}

/// Encode a single (non-optional) value of `ty` bound to `val_expr` into a
/// `JsonValue` expression. A `List<T>` needs a loop (built via
/// `Json.array()` + `JsonValue.push`), so — like decoding — it materializes
/// into a fresh `var` via statements written to `out` (indented by
/// `indent`, so it reads correctly whether called at a model's top level or
/// from inside a `match` arm).
fn emit_encode_value(out: &mut String, ty: &FieldTy, val_expr: &str, hint: &str, indent: &str) -> String {
    match ty {
        FieldTy::List(inner) => {
            let acc = format!("__jarr{hint}");
            let item = format!("__item{hint}");
            // Json.array()/JsonValue.push mutate the array handle in place
            // (JsonValue.push returns Unit) — unlike List.push, which
            // returns a *new* list — so the accumulator binding itself
            // never needs reassigning.
            writeln!(out, "{indent}val {acc} = Json.array()").unwrap();
            writeln!(out, "{indent}for {item} in {val_expr} {{").unwrap();
            let elem_expr = match inner.as_ref() {
                FieldTy::List(_) => emit_encode_value(out, inner, &item, &format!("{hint}_e"), &format!("{indent}    ")),
                other => other.encode_expr(&item),
            };
            writeln!(out, "{indent}    JsonValue.push({acc}, {elem_expr})").unwrap();
            writeln!(out, "{indent}}}").unwrap();
            acc
        }
        other => other.encode_expr(val_expr),
    }
}

// ------------------------------------------------------------------ //
// Endpoint codegen
// ------------------------------------------------------------------ //

fn emit_endpoint(out: &mut String, ep: &Endpoint, base_url: &str, models: &[Model]) {
    writeln!(out, "// {} {}", ep.method.to_uppercase(), ep.path).unwrap();

    let resp_ty = ep.response.as_deref()
        .map(|r| FieldTy::parse(r, models).expect("validated"))
        .unwrap_or(FieldTy::Unit);
    let body_ty = ep.body.as_ref().map(|b| FieldTy::parse(&b.ty, models).expect("validated"));

    // Signature: path/query params, then `body` last if present.
    let mut params: Vec<String> = Vec::new();
    for p in &ep.path_params {
        let ty = FieldTy::parse(&p.ty, models).expect("validated");
        params.push(format!("{}: {}{}", p.name, ty.certo_ty(), if p.optional { "?" } else { "" }));
    }
    for p in &ep.query_params {
        let ty = FieldTy::parse(&p.ty, models).expect("validated");
        params.push(format!("{}: {}{}", p.name, ty.certo_ty(), if p.optional { "?" } else { "" }));
    }
    if let Some(bt) = &body_ty {
        params.push(format!("body: {}", bt.certo_ty()));
    }
    // Http.get/post/put/delete/request are Io-effectful (per
    // certo_stdlib::seed_stdlib_effects) — not Async, which is a distinct
    // Certo effect (declared-async concurrency, spawn/await) unrelated to
    // "this function performs I/O"; the previous generator's `[async, io]`
    // claimed an effect these calls never actually require.
    let ret_ty = if matches!(resp_ty, FieldTy::Unit) {
        "Result<Unit, Text>".to_string()
    } else {
        format!("Result<{}, Text>", resp_ty.certo_ty())
    };
    writeln!(out, "pub fn {}({}): {} [io] = {{", ep.name, params.join(", "), ret_ty).unwrap();

    // Query string: built as a list of "key=value" fragments (required
    // params always included, optional ones only when present) then
    // joined — Http.get/delete/post/put only ever take a single URL
    // string, there's no separate structured query-params argument.
    let has_query = !ep.query_params.is_empty();
    if has_query {
        writeln!(out, "    var __qs = List.empty()").unwrap();
        for p in &ep.query_params {
            let ty = FieldTy::parse(&p.ty, models).expect("validated");
            if p.optional {
                writeln!(out, "    match {} {{", p.name).unwrap();
                writeln!(out, "        Some(__v) => {{ __qs = List.push(__qs, \"{}=\" ++ {}) }}",
                    p.name, ty.to_text_expr("__v")).unwrap();
                // `Unit` is not a real value literal in Certo (confirmed
            // against typeck: "undefined name `Unit`" — the GUIDE.md
            // example that suggests it is itself stale/wrong). An empty
            // block `{}` genuinely infers as `Ty::Unit` by default
            // (`infer_block`'s `last_ty` starts at `Ty::Unit` and only
            // changes if there's at least one statement), so that's the
            // real way to write a bare Unit value.
            writeln!(out, "        None => {{}}").unwrap();
                writeln!(out, "    }}").unwrap();
            } else {
                writeln!(out, "    __qs = List.push(__qs, \"{}=\" ++ {})", p.name, ty.to_text_expr(&p.name)).unwrap();
            }
        }
        writeln!(out, "    val __qsStr = if List.len(__qs) == 0 then \"\" else \"?\" ++ Text.join(__qs, \"&\")").unwrap();
    }

    let path_expr = build_path_expr(base_url, &ep.path, &ep.path_params, models);
    if has_query {
        writeln!(out, "    val __url = ({path_expr}) ++ __qsStr").unwrap();
    } else {
        writeln!(out, "    val __url = {path_expr}").unwrap();
    }

    // The actual HTTP call, using the real stdlib signatures.
    let method = ep.method.to_lowercase();
    match method.as_str() {
        "get" | "delete" => {
            writeln!(out, "    val __resp = Http.{}(__url)", method).unwrap();
        }
        "post" | "put" => {
            let (body_expr, content_type) = match (&body_ty, &ep.body) {
                (Some(FieldTy::Model(name)), Some(bd)) => (format!("Json.stringify({}(body))", encoder_fn_name(name)), bd.content_type.clone()),
                (Some(FieldTy::Text), Some(bd)) => ("body".to_string(), bd.content_type.clone()),
                (Some(other), Some(bd)) => (other.to_text_expr("body"), bd.content_type.clone()),
                _ => ("\"\"".to_string(), "application/json".to_string()),
            };
            // Real order is (url, body, content_type) — matches the actual
            // C implementation and docs/STDLIB-QUICKREF.md; seed.rs's `pm!`
            // metadata previously listed them the other way around (a real,
            // now-fixed bug — see its own comment) which didn't affect this
            // positional call, but did make the wrong order look "declared"
            // if you went looking. Confirmed directly (not theorized): a
            // model-typed body sent with the swapped order arrived at the
            // server as an empty/garbage body while verifying this item.
            writeln!(out, "    val __resp = Http.{}(__url, {}, \"{}\")", method, body_expr, content_type).unwrap();
        }
        other => {
            let body_expr = match (&body_ty, &ep.body) {
                (Some(FieldTy::Model(name)), Some(_)) => format!("Json.stringify({}(body))", encoder_fn_name(name)),
                (Some(FieldTy::Text), Some(_)) => "body".to_string(),
                (Some(t), Some(_)) => t.to_text_expr("body"),
                _ => "\"\"".to_string(),
            };
            writeln!(out, "    val __resp = Http.request(\"{}\", __url, List.empty(), {})", other.to_uppercase(), body_expr).unwrap();
        }
    }

    // Decode the response, or just propagate ok/err for a Unit response.
    writeln!(out, "    if HttpResponse.ok(__resp) then {{").unwrap();
    match &resp_ty {
        FieldTy::Unit => writeln!(out, "        Ok({{}})").unwrap(), // see the `None => {{}}` comment above re: Unit not being a real value literal
        FieldTy::Model(name) => writeln!(out, "        Ok({}(Json.parse(HttpResponse.body(__resp))))", decoder_fn_name(name)).unwrap(),
        FieldTy::List(_) => {
            let decoded = emit_decode_value(out, &resp_ty, "Json.parse(HttpResponse.body(__resp))", "__resp");
            writeln!(out, "        Ok({decoded})").unwrap();
        }
        FieldTy::Text => writeln!(out, "        Ok(HttpResponse.body(__resp))").unwrap(),
        prim => writeln!(out, "        Ok({})", prim.decode_expr("Json.parse(HttpResponse.body(__resp))")).unwrap(),
    }
    writeln!(out, "    }} else {{").unwrap();
    writeln!(out, "        Err(HttpResponse.body(__resp))").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
}

/// Build a Certo string expression for the full URL path (base URL + path,
/// with `{param}` placeholders substituted) — query string is appended
/// separately by the caller. Path placeholders are converted to `Text` via
/// the real stdlib conversion function for their declared type
/// (`intToText`/`floatToText`/`boolToText` — `.toString()` doesn't exist
/// anywhere in Certo's stdlib, a real bug in the pre-item-93 generator).
fn build_path_expr(base_url: &str, path: &str, path_params: &[Param], models: &[Model]) -> String {
    let base = base_url.trim_end_matches('/');
    let mut parts: Vec<String> = vec![format!("\"{}\"", base)];
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        if open > 0 {
            let lit = &rest[..open];
            append_literal(&mut parts, lit);
        }
        rest = &rest[open + 1..];
        if let Some(close) = rest.find('}') {
            let param_name = &rest[..close];
            let p = path_params.iter().find(|p| p.name == param_name);
            let conv = match p {
                Some(p) => {
                    let ty = FieldTy::parse(&p.ty, models).expect("validated");
                    ty.to_text_expr(param_name)
                }
                None => param_name.to_owned(), // schema/path mismatch — leave as-is, caller's problem
            };
            parts.push(conv);
            rest = &rest[close + 1..];
        }
    }
    if !rest.is_empty() {
        append_literal(&mut parts, rest);
    }
    parts.join(" ++ ")
}

fn append_literal(parts: &mut Vec<String>, lit: &str) {
    // Merge into the previous part if it's also a literal, to avoid
    // `"a" ++ "b"` where `"ab"` reads better — purely cosmetic.
    if let Some(last) = parts.last_mut() {
        if last.starts_with('"') && last.ends_with('"') {
            let merged = format!("{}{}\"", &last[..last.len() - 1], lit);
            *last = merged;
            return;
        }
    }
    parts.push(format!("\"{}\"", lit));
}

// ------------------------------------------------------------------ //
// Tests
// ------------------------------------------------------------------ //

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = r#"
    {
      "module":   "UserApi",
      "base_url": "https://api.example.com",
      "models": {
        "User": { "fields": [
          { "name": "id", "ty": "Int" },
          { "name": "name", "ty": "Text" },
          { "name": "email", "ty": "Text", "optional": true }
        ] },
        "CreateUserRequest": { "fields": [
          { "name": "name", "ty": "Text" }
        ] }
      },
      "endpoints": [
        {
          "name":        "getUser",
          "method":      "GET",
          "path":        "/users/{id}",
          "path_params": [{ "name": "id", "ty": "Int" }],
          "response":    "User"
        },
        {
          "name":      "listUserNames",
          "method":    "GET",
          "path":      "/users",
          "query_params": [{ "name": "page", "ty": "Int", "optional": true }],
          "response":  "List<Text>"
        },
        {
          "name":     "createUser",
          "method":   "POST",
          "path":     "/users",
          "body":     { "ty": "CreateUserRequest" },
          "response": "User"
        },
        {
          "name":        "deleteUser",
          "method":      "DELETE",
          "path":        "/users/{id}",
          "path_params": [{ "name": "id", "ty": "Int" }],
          "response":    null
        }
      ]
    }
    "#;

    fn schema() -> RestSchema { parse_schema(SCHEMA).expect("parse") }

    #[test]
    fn parse_schema_ok() {
        let s = schema();
        assert_eq!(s.module, "UserApi");
        assert_eq!(s.endpoints.len(), 4);
        assert_eq!(s.models.len(), 2);
    }

    #[test]
    fn module_declaration_present() {
        let src = generate_client(&schema());
        assert!(src.starts_with("module UserApi"), "wrong module header\n{}", src);
    }

    #[test]
    fn model_type_and_codecs_generated() {
        let src = generate_client(&schema());
        assert!(src.contains("type User = { id: Int, name: Text, email: Text? }"), "{}", src);
        assert!(src.contains("fn __decodeUser(jv: JsonValue): User"), "{}", src);
        assert!(src.contains("fn __encodeUser(v: User): JsonValue"), "{}", src);
    }

    #[test]
    fn optional_composite_field_encodes_with_statements_inside_the_match_arm() {
        // Regression test for a real bug: an optional field whose type is
        // itself composite (List<T>?, not just a scalar) needs
        // emit_encode_value's statements (the array-building loop) placed
        // *inside* `Some(x) => { ... }`, not spliced in before the match's
        // arms — the latter is a syntax error, confirmed by actually
        // compiling generated output for a model with an optional
        // List<Text> field (this item's own hand-written test schema's
        // `tags` field was never optional, so this path went untested
        // until an OpenAPI spec that didn't list `tags` under `required`
        // exercised it).
        let json = r#"{
            "module": "M", "base_url": "http://x",
            "models": { "Widget": { "fields": [
                { "name": "tags", "ty": "List<Text>", "optional": true }
            ] } },
            "endpoints": []
        }"#;
        let schema = parse_schema(json).unwrap();
        let src = generate_client(&schema);
        assert!(src.contains("Some(x) => {"), "expected a block arm, not an inline one\n{}", src);
        // The array-building loop must appear strictly between "Some(x) => {"
        // and its own closing "}", not before "match v.tags {" at all.
        let match_start = src.find("match v.tags {").expect("match not found");
        let some_arm = src[match_start..].find("Some(x) => {").expect("Some arm not found") + match_start;
        let loop_pos = src[match_start..].find("Json.array()").expect("array loop not found") + match_start;
        assert!(loop_pos > some_arm, "array-building loop must be inside the Some arm, not before it\n{}", src);
    }

    #[test]
    fn endpoint_uses_real_http_get_and_io_effect_not_async() {
        let src = generate_client(&schema());
        assert!(src.contains("Http.get(__url)"), "{}", src);
        assert!(src.contains("[io] = {"), "should use [io], not the nonexistent [async, io]\n{}", src);
        assert!(!src.contains("[async"), "async isn't a real effect Http.* requires\n{}", src);
    }

    #[test]
    fn path_param_uses_real_conversion_function() {
        let src = generate_client(&schema());
        assert!(src.contains("intToText(id)"), "toString() doesn't exist in Certo stdlib\n{}", src);
        assert!(!src.contains(".toString()"), "{}", src);
    }

    #[test]
    fn post_encodes_body_via_generated_encoder_and_real_http_post_signature() {
        let src = generate_client(&schema());
        // Real Http.post order is (url, body, content_type) — confirmed
        // against the actual C implementation, not just seed.rs's metadata.
        assert!(src.contains("Http.post(__url, Json.stringify(__encodeCreateUserRequest(body)), \"application/json\")"), "{}", src);
    }

    #[test]
    fn response_model_decoded_via_generated_decoder_wrapped_in_result() {
        let src = generate_client(&schema());
        assert!(src.contains("Result<User, Text>"), "{}", src);
        assert!(src.contains("Ok(__decodeUser(Json.parse(HttpResponse.body(__resp))))"), "{}", src);
        assert!(src.contains("Err(HttpResponse.body(__resp))"), "{}", src);
    }

    #[test]
    fn list_response_decodes_via_loop() {
        let src = generate_client(&schema());
        assert!(src.contains("Result<List<Text>, Text>"), "{}", src);
        assert!(src.contains("List.push("), "{}", src);
        assert!(src.contains("range(0, JsonValue.length("), "{}", src);
    }

    #[test]
    fn list_response_parses_body_exactly_once_not_per_element() {
        // A naive inline-expression decode would re-embed
        // Json.parse(HttpResponse.body(...)) once for the length check and
        // again on every loop iteration — this asserts it's bound to a
        // local exactly once instead, scoped to just `listUserNames`'s own
        // body (getUser/createUser legitimately produce the same
        // substring once each too, since they decode a plain, non-list
        // model — the whole-file count isn't the right thing to assert).
        let src = generate_client(&schema());
        let start = src.find("pub fn listUserNames").expect("listUserNames not found");
        let body = &src[start..start + src[start..].find("\n}\n").unwrap()];
        assert_eq!(body.matches("Json.parse(HttpResponse.body(__resp))").count(), 1, "{}", body);
    }

    #[test]
    fn list_of_model_response_is_rejected_struct_cant_box_into_list_slot() {
        // The same confirmed, pre-existing compiler gap item 124 found for
        // property-test generation's List<Record> — a model is a real
        // struct passed by value, and List<T>'s element slot is a
        // pointer-sized void* boxed via crates/codegen's box_value
        // convention, which can't hold one. Confirmed directly by actually
        // compiling generated C for a List<Model> response before adding
        // this rejection: "operand of type 'User' where arithmetic or
        // pointer type is required" from certo_list_push's boxing cast.
        let bad = r#"{"module":"M","base_url":"http://x","models":{
            "User": { "fields": [{ "name": "id", "ty": "Int" }] }
        },"endpoints":[
            {"name":"f","method":"GET","path":"/f","response":"List<User>"}
        ]}"#;
        let err = parse_schema(bad).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("List<User>"), "{msg}");
        assert!(msg.contains("f"), "should name the endpoint: {msg}");
    }

    #[test]
    fn delete_returns_unit_result() {
        let src = generate_client(&schema());
        assert!(src.contains("Result<Unit, Text>"), "{}", src);
        // `Unit` is not a real value literal in Certo (confirmed against
        // the real type-checker: E0206 undefined name) — an empty block
        // `{}` is the real way to produce a Ty::Unit value.
        assert!(src.contains("Ok({})"), "{}", src);
    }

    #[test]
    fn optional_query_param_builds_conditional_fragment() {
        let src = generate_client(&schema());
        assert!(src.contains("page: Int?"), "{}", src);
        assert!(src.contains("Some(__v) => { __qs = List.push(__qs, \"page=\" ++ intToText(__v)) }"), "{}", src);
        assert!(src.contains("None => {}"), "{}", src);
    }

    #[test]
    fn invalid_json_gives_schema_error() {
        let err = parse_schema("not json").unwrap_err();
        assert!(matches!(err, FfiError::SchemaError(_)));
    }

    #[test]
    fn unknown_response_type_is_a_clear_schema_error() {
        let bad = r#"{"module":"M","base_url":"http://x","endpoints":[
            {"name":"f","method":"GET","path":"/f","response":"NotAModel"}
        ]}"#;
        let err = parse_schema(bad).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("NotAModel"), "{msg}");
        assert!(msg.contains("f"), "should name the endpoint: {msg}");
    }

    #[test]
    fn base_url_in_generated_code() {
        let src = generate_client(&schema());
        assert!(src.contains("api.example.com"), "missing base URL\n{}", src);
    }

    #[test]
    fn field_ty_parse_handles_nested_lists() {
        let models = vec![];
        assert_eq!(FieldTy::parse("List<List<Int>>", &models), Ok(FieldTy::List(Box::new(FieldTy::List(Box::new(FieldTy::Int))))));
    }
}
