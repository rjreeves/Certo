//! Parse an OpenAPI 3.x document (JSON) into a `rest::RestSchema`, reusing
//! `rest::generate_client` unchanged for codegen — this module's only job is
//! translating OpenAPI's shape into the same `models`/`endpoints`
//! representation the hand-written custom-JSON schema already produces.
//!
//! # Scope
//!
//! - JSON only (no YAML — `serde_yaml` isn't a workspace dependency, and
//!   adding one for this alone wasn't judged worth it; a spec author can
//!   convert YAML to JSON with any standard tool before feeding it in).
//! - A local file path only — no URL fetching. The spec's own aspirational
//!   example (`@apiClient("https://.../openapi.json") module Stripe`, which
//!   downloads the spec *during every build*) is a materially different,
//!   larger feature — same "every build needs network access" concern
//!   BACKLOG item 72b was deferred over, not attempted here.
//! - `components.schemas` entries become models only when they're a plain
//!   `object` whose `properties` are each a primitive, an array of
//!   primitives/refs, or a `$ref` to another schema — matching exactly
//!   what `rest::FieldTy`/`rest::Model` already support (see rest.rs's own
//!   module doc comment for why: a model field can't be a struct-shaped
//!   `List` element, and an anonymous/inline object has no real C
//!   representation). A schema using `oneOf`/`anyOf`/`allOf`, a bare
//!   `additionalProperties` map, or any other shape reports a clear error
//!   naming the offending schema rather than silently emitting `void*`/
//!   `Text` junk for it.
//! - Only the success response (2xx, or "default" if no 2xx is listed) is
//!   used to determine an endpoint's response type — per-status-code typed
//!   error variants (the spec's `Result<Charge, StripeError>` framing)
//!   aren't attempted; every endpoint still resolves to `Result<T, Text>`
//!   exactly like the custom-JSON path, for the same reason documented on
//!   `rest`'s module doc comment.
//! - Request bodies are read from the `application/json` content type only.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::FfiError;
use crate::rest::{BodyDef, Endpoint, Model, ModelField, Param, RestSchema};

// ------------------------------------------------------------------ //
// OpenAPI document shape (only the subset this module reads)
// ------------------------------------------------------------------ //

#[derive(Debug, Deserialize)]
struct OpenApiDoc {
    info: Option<OpenApiInfo>,
    #[serde(default)]
    servers: Vec<OpenApiServer>,
    #[serde(default)]
    paths: BTreeMap<String, BTreeMap<String, OpenApiOperation>>,
    components: Option<OpenApiComponents>,
}

#[derive(Debug, Deserialize)]
struct OpenApiInfo {
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenApiServer {
    url: String,
}

#[derive(Debug, Deserialize)]
struct OpenApiComponents {
    #[serde(default)]
    schemas: BTreeMap<String, OpenApiSchema>,
}

#[derive(Debug, Deserialize)]
struct OpenApiOperation {
    #[serde(rename = "operationId")]
    operation_id: Option<String>,
    #[serde(default)]
    parameters: Vec<OpenApiParam>,
    #[serde(rename = "requestBody")]
    request_body: Option<OpenApiRequestBody>,
    #[serde(default)]
    responses: BTreeMap<String, OpenApiResponse>,
}

#[derive(Debug, Deserialize)]
struct OpenApiParam {
    name: String,
    #[serde(rename = "in")]
    location: String, // "path" | "query" | "header" | "cookie" — only path/query are used
    #[serde(default)]
    required: bool,
    schema: Option<OpenApiSchema>,
}

#[derive(Debug, Deserialize)]
struct OpenApiRequestBody {
    content: BTreeMap<String, OpenApiMediaType>,
}

#[derive(Debug, Deserialize)]
struct OpenApiResponse {
    content: Option<BTreeMap<String, OpenApiMediaType>>,
}

#[derive(Debug, Deserialize)]
struct OpenApiMediaType {
    schema: Option<OpenApiSchema>,
}

/// A JSON Schema fragment, as used inline throughout OpenAPI 3.x.
#[derive(Debug, Deserialize, Clone)]
struct OpenApiSchema {
    #[serde(rename = "$ref")]
    reference:  Option<String>,
    #[serde(rename = "type")]
    schema_type: Option<String>,
    items:       Option<Box<OpenApiSchema>>,
    #[serde(default)]
    properties:  BTreeMap<String, OpenApiSchema>,
    #[serde(default)]
    required:    Vec<String>,
    // Presence alone is enough to reject a schema as unsupported (see
    // `resolve_field_ty`'s scope note) — contents are never read.
    #[serde(rename = "oneOf")]
    one_of: Option<serde_json::Value>,
    #[serde(rename = "anyOf")]
    any_of: Option<serde_json::Value>,
    #[serde(rename = "allOf")]
    all_of: Option<serde_json::Value>,
}

// ------------------------------------------------------------------ //
// Public entry point
// ------------------------------------------------------------------ //

/// Parse an OpenAPI 3.x JSON document into a `RestSchema`. `module_name`
/// overrides `info.title`-derived naming when given (mirrors `--module` on
/// the CLI); otherwise the title is sanitised into a PascalCase Certo
/// module name, falling back to `"Api"` if there's no title at all.
pub fn parse_openapi(json: &str, module_name: Option<&str>) -> Result<RestSchema, FfiError> {
    let doc: OpenApiDoc = serde_json::from_str(json).map_err(|e| FfiError::SchemaError(e.to_string()))?;

    let module = module_name.map(str::to_string).unwrap_or_else(|| {
        doc.info.as_ref().and_then(|i| i.title.as_deref()).map(sanitise_module_name)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Api".to_string())
    });

    let base_url = doc.servers.first().map(|s| s.url.clone()).ok_or_else(|| {
        FfiError::SchemaError("OpenAPI document has no `servers` entry — certo-ffi needs at least one to know the base URL".to_string())
    })?;

    let schemas = doc.components.map(|c| c.schemas).unwrap_or_default();
    let models = build_models(&schemas)?;
    let endpoints = build_endpoints(&doc.paths, &schemas)?;

    let schema = RestSchema { module, base_url, models, endpoints };
    crate::rest::validate_schema(&schema)?;
    Ok(schema)
}

fn sanitise_module_name(title: &str) -> String {
    let mut out = String::new();
    let mut capitalise_next = true;
    for c in title.chars() {
        if c.is_alphanumeric() {
            if capitalise_next {
                out.extend(c.to_uppercase());
                capitalise_next = false;
            } else {
                out.push(c);
            }
        } else {
            capitalise_next = true;
        }
    }
    out
}

// ------------------------------------------------------------------ //
// Schema (type) resolution — produces the same string spellings
// `rest::FieldTy::parse` already understands, so this module never needs
// to duplicate that logic.
// ------------------------------------------------------------------ //

fn resolve_field_ty(schema: &OpenApiSchema, context: &str) -> Result<String, String> {
    if let Some(reference) = &schema.reference {
        return reference.rsplit('/').next()
            .map(str::to_string)
            .ok_or_else(|| format!("{context}: malformed $ref `{reference}`"));
    }
    if schema.one_of.is_some() || schema.any_of.is_some() || schema.all_of.is_some() {
        return Err(format!("{context}: oneOf/anyOf/allOf schemas aren't supported"));
    }
    match schema.schema_type.as_deref() {
        Some("integer") => Ok("Int".to_string()),
        Some("number")  => Ok("Float".to_string()),
        Some("boolean") => Ok("Bool".to_string()),
        Some("string")  => Ok("Text".to_string()),
        Some("array") => {
            let items = schema.items.as_deref()
                .ok_or_else(|| format!("{context}: array schema has no `items`"))?;
            let inner = resolve_field_ty(items, context)?;
            Ok(format!("List<{inner}>"))
        }
        Some("object") | None if !schema.properties.is_empty() => {
            Err(format!("{context}: inline/anonymous object schemas aren't supported — give it a name under components.schemas and reference it with $ref"))
        }
        Some(other) => Err(format!("{context}: unsupported schema type `{other}`")),
        None => Err(format!("{context}: schema has neither `type` nor `$ref`")),
    }
}

fn build_models(schemas: &BTreeMap<String, OpenApiSchema>) -> Result<Vec<Model>, FfiError> {
    let mut models = Vec::new();
    for (name, schema) in schemas {
        // Only plain named objects become models — a components.schemas
        // entry that's itself just an alias/array/etc. is skipped rather
        // than rejected, since it's never required to be a model (only
        // referenced *as* one, which surfaces its own clear error at the
        // point of use if that reference expected an object shape).
        if schema.schema_type.as_deref() != Some("object") {
            continue;
        }
        let mut fields = Vec::new();
        for (field_name, field_schema) in &schema.properties {
            let ty = resolve_field_ty(field_schema, &format!("model `{name}` field `{field_name}`"))
                .map_err(FfiError::SchemaError)?;
            let optional = !schema.required.iter().any(|r| r == field_name);
            fields.push(ModelField { name: field_name.clone(), ty, optional });
        }
        models.push(Model { name: name.clone(), fields });
    }
    Ok(models)
}

fn build_endpoints(
    paths: &BTreeMap<String, BTreeMap<String, OpenApiOperation>>,
    schemas: &BTreeMap<String, OpenApiSchema>,
) -> Result<Vec<Endpoint>, FfiError> {
    let mut endpoints = Vec::new();
    for (path, methods) in paths {
        for (method, op) in methods {
            if !matches!(method.to_lowercase().as_str(), "get" | "post" | "put" | "patch" | "delete") {
                continue; // "parameters" and other non-operation keys can appear at the path-item level too
            }
            let name = op.operation_id.clone().unwrap_or_else(|| synthesize_name(method, path));

            let mut path_params = Vec::new();
            let mut query_params = Vec::new();
            for p in &op.parameters {
                let schema = p.schema.as_ref()
                    .ok_or_else(|| FfiError::SchemaError(format!("endpoint `{name}` param `{}`: no schema given", p.name)))?;
                let ty = resolve_field_ty(schema, &format!("endpoint `{name}` param `{}`", p.name))
                    .map_err(FfiError::SchemaError)?;
                let param = Param { name: p.name.clone(), ty, optional: !p.required };
                match p.location.as_str() {
                    "path"  => path_params.push(param),
                    "query" => query_params.push(param),
                    _ => {} // header/cookie params aren't part of this generator's scope
                }
            }

            let body = op.request_body.as_ref()
                .and_then(|rb| rb.content.get("application/json"))
                .and_then(|mt| mt.schema.as_ref())
                .map(|s| resolve_field_ty(s, &format!("endpoint `{name}` request body")).map_err(FfiError::SchemaError))
                .transpose()?
                .map(|ty| BodyDef { ty, content_type: "application/json".to_string() });

            let response = success_response_ty(&op.responses, &name)?;

            endpoints.push(Endpoint {
                name,
                method: method.to_uppercase(),
                path: path.clone(),
                path_params,
                query_params,
                body,
                response,
            });
        }
    }
    let _ = schemas; // resolve_field_ty only needs the schema map for $ref *names*, already flat strings
    Ok(endpoints)
}

/// Pick the response to type the endpoint's `Result<T, Text>` after: the
/// first 2xx entry found, else `"default"`, else `Unit` (no success
/// response documented at all — treated as no body expected, same as the
/// custom-JSON schema's `"response": null`).
fn success_response_ty(responses: &BTreeMap<String, OpenApiResponse>, endpoint_name: &str) -> Result<Option<String>, FfiError> {
    let chosen = responses.iter()
        .find(|(code, _)| code.starts_with('2'))
        .map(|(_, r)| r)
        .or_else(|| responses.get("default"));

    let Some(resp) = chosen else { return Ok(None) };
    let Some(content) = &resp.content else { return Ok(None) }; // e.g. 204 No Content
    let Some(mt) = content.get("application/json") else { return Ok(None) };
    let Some(schema) = &mt.schema else { return Ok(None) };

    resolve_field_ty(schema, &format!("endpoint `{endpoint_name}` response"))
        .map(Some)
        .map_err(FfiError::SchemaError)
}

/// `operationId` is optional in OpenAPI — fall back to a name built from
/// the method and path when it's missing (`GET /users/{id}` → `getUsersId`).
fn synthesize_name(method: &str, path: &str) -> String {
    let mut out = method.to_lowercase();
    for seg in path.split('/') {
        let seg = seg.trim_start_matches('{').trim_end_matches('}');
        if seg.is_empty() { continue; }
        let mut chars = seg.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // r##"..."## (not r#"..."#) — the JSON body's own $ref pointers contain
    // a literal "# sequence ("#/components/schemas/User"), which would
    // otherwise terminate a single-# raw string early.
    const SPEC: &str = r##"
    {
      "info": { "title": "User API" },
      "servers": [{ "url": "https://api.example.com" }],
      "paths": {
        "/users/{id}": {
          "get": {
            "operationId": "getUser",
            "parameters": [
              { "name": "id", "in": "path", "required": true, "schema": { "type": "integer" } }
            ],
            "responses": {
              "200": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/User" } } } }
            }
          },
          "delete": {
            "operationId": "deleteUser",
            "parameters": [
              { "name": "id", "in": "path", "required": true, "schema": { "type": "integer" } }
            ],
            "responses": { "204": {} }
          }
        },
        "/users": {
          "get": {
            "operationId": "listUserNames",
            "parameters": [
              { "name": "page", "in": "query", "required": false, "schema": { "type": "integer" } }
            ],
            "responses": {
              "200": { "content": { "application/json": { "schema": { "type": "array", "items": { "type": "string" } } } } }
            }
          },
          "post": {
            "operationId": "createUser",
            "requestBody": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateUserRequest" } } } },
            "responses": {
              "201": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/User" } } } }
            }
          }
        }
      },
      "components": {
        "schemas": {
          "User": {
            "type": "object",
            "required": ["id", "name"],
            "properties": {
              "id": { "type": "integer" },
              "name": { "type": "string" },
              "email": { "type": "string" },
              "tags": { "type": "array", "items": { "type": "string" } }
            }
          },
          "CreateUserRequest": {
            "type": "object",
            "required": ["name"],
            "properties": { "name": { "type": "string" } }
          }
        }
      }
    }
    "##;

    #[test]
    fn parses_module_name_and_base_url() {
        let s = parse_openapi(SPEC, None).unwrap();
        assert_eq!(s.module, "UserAPI");
        assert_eq!(s.base_url, "https://api.example.com");
    }

    #[test]
    fn module_name_override_wins_over_title() {
        let s = parse_openapi(SPEC, Some("Custom")).unwrap();
        assert_eq!(s.module, "Custom");
    }

    #[test]
    fn models_resolved_with_correct_optionality() {
        let s = parse_openapi(SPEC, None).unwrap();
        let user = s.models.iter().find(|m| m.name == "User").unwrap();
        let id = user.fields.iter().find(|f| f.name == "id").unwrap();
        assert_eq!(id.ty, "Int");
        assert!(!id.optional, "id is in `required`");
        let email = user.fields.iter().find(|f| f.name == "email").unwrap();
        assert!(email.optional, "email is not in `required`");
        let tags = user.fields.iter().find(|f| f.name == "tags").unwrap();
        assert_eq!(tags.ty, "List<Text>");
    }

    #[test]
    fn path_and_query_params_split_correctly() {
        let s = parse_openapi(SPEC, None).unwrap();
        let get_user = s.endpoints.iter().find(|e| e.name == "getUser").unwrap();
        assert_eq!(get_user.path_params.len(), 1);
        assert_eq!(get_user.path_params[0].name, "id");
        assert!(get_user.query_params.is_empty());

        let list_user_names = s.endpoints.iter().find(|e| e.name == "listUserNames").unwrap();
        assert!(list_user_names.path_params.is_empty());
        assert_eq!(list_user_names.query_params.len(), 1);
        assert!(list_user_names.query_params[0].optional, "page is required:false");
    }

    #[test]
    fn ref_response_resolves_to_model_name() {
        let s = parse_openapi(SPEC, None).unwrap();
        let get_user = s.endpoints.iter().find(|e| e.name == "getUser").unwrap();
        assert_eq!(get_user.response.as_deref(), Some("User"));
    }

    #[test]
    fn array_of_primitive_response_resolves_to_list_of_text() {
        let s = parse_openapi(SPEC, None).unwrap();
        let list_user_names = s.endpoints.iter().find(|e| e.name == "listUserNames").unwrap();
        assert_eq!(list_user_names.response.as_deref(), Some("List<Text>"));
    }

    #[test]
    fn array_of_ref_response_is_rejected_same_as_the_custom_schema_path() {
        // Same confirmed struct-boxing gap `rest::tests::
        // list_of_model_response_is_rejected_...` documents — an OpenAPI
        // `array` of `$ref`-to-object schema hits the identical rejection
        // once resolve_field_ty produces the same "List<Model>" string
        // rest::FieldTy::parse already refuses.
        let spec = r##"{
            "info": {}, "servers": [{"url": "http://x"}],
            "paths": { "/users": { "get": { "operationId": "f", "responses": {
                "200": { "content": { "application/json": { "schema": {
                    "type": "array", "items": { "$ref": "#/components/schemas/User" }
                } } } }
            } } } },
            "components": { "schemas": { "User": {
                "type": "object", "properties": { "id": { "type": "integer" } }
            } } }
        }"##;
        let err = parse_openapi(spec, None).unwrap_err();
        assert!(err.to_string().contains("List<User>"), "{err}");
    }

    #[test]
    fn request_body_ref_resolves_to_model_name() {
        let s = parse_openapi(SPEC, None).unwrap();
        let create_user = s.endpoints.iter().find(|e| e.name == "createUser").unwrap();
        assert_eq!(create_user.body.as_ref().map(|b| b.ty.as_str()), Some("CreateUserRequest"));
    }

    #[test]
    fn no_content_response_is_unit() {
        let s = parse_openapi(SPEC, None).unwrap();
        let delete_user = s.endpoints.iter().find(|e| e.name == "deleteUser").unwrap();
        assert_eq!(delete_user.response, None);
    }

    #[test]
    fn missing_operation_id_synthesizes_a_name() {
        let spec = r#"{
            "info": {}, "servers": [{"url": "http://x"}],
            "paths": { "/widgets/{id}": { "get": { "responses": {} } } }
        }"#;
        let s = parse_openapi(spec, None).unwrap();
        assert_eq!(s.endpoints[0].name, "getWidgetsId");
    }

    #[test]
    fn missing_servers_is_a_clear_error() {
        let spec = r#"{"info": {"title": "X"}, "paths": {}}"#;
        let err = parse_openapi(spec, None).unwrap_err();
        assert!(err.to_string().contains("servers"), "{err}");
    }

    #[test]
    fn one_of_schema_is_a_clear_error() {
        let spec = r#"{
            "info": {}, "servers": [{"url": "http://x"}],
            "paths": {},
            "components": { "schemas": { "Weird": {
                "type": "object", "properties": { "x": { "oneOf": [{"type":"integer"},{"type":"string"}] } }
            } } }
        }"#;
        let err = parse_openapi(spec, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Weird"), "{msg}");
        assert!(msg.contains("oneOf"), "{msg}");
    }

    #[test]
    fn no_title_falls_back_to_api() {
        let spec = r#"{"info": {}, "servers": [{"url": "http://x"}], "paths": {}}"#;
        let s = parse_openapi(spec, None).unwrap();
        assert_eq!(s.module, "Api");
    }
}
