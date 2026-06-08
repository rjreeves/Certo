use certo_ast::module::Module;
use certo_ast::span::Span;
use certo_ast::decl::Decl;
use certo_resolve::ResolveError;
use certo_typeck::TypeError;
use crate::pos::span_contains;

/// A single parsed + analysed document.
pub struct Analysis {
    pub src:           String,
    pub module:        Option<Module>,
    pub parse_errors:  Vec<(String, Span)>,
    pub resolve_errors: Vec<ResolveError>,
    pub type_errors:   Vec<TypeError>,
    /// Top-level symbol table: name → (definition span, human-readable type hint).
    pub symbols:       Vec<Symbol>,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name:     String,
    pub def_span: Span,
    pub kind:     SymbolKind,
    pub detail:   String, // hover text
}

#[derive(Debug, Clone)]
pub enum SymbolKind { Function, Const, Type, Param }

impl Analysis {
    pub fn run(src: &str) -> Self {
        let mut parse_errors = Vec::new();
        let mut resolve_errors = Vec::new();
        let mut type_errors = Vec::new();
        let mut symbols = Vec::new();

        let module = match certo_parser::parse(src) {
            Ok(m) => {
                // Name resolution
                if let Err(errs) = certo_resolve::resolve(&m) {
                    resolve_errors = errs;
                }
                // Type checking
                if let Err(errs) = certo_typeck::check_module(&m) {
                    type_errors = errs;
                }
                // Build symbol table from AST
                collect_symbols(&m, &mut symbols);
                Some(m)
            }
            Err(errs) => {
                for e in errs {
                    parse_errors.push((format!("{}", e), e.span));
                }
                None
            }
        };

        Analysis { src: src.to_string(), module, parse_errors, resolve_errors, type_errors, symbols }
    }

    /// Find the innermost symbol whose def_span contains `offset`.
    pub fn symbol_at(&self, offset: u32) -> Option<&Symbol> {
        self.symbols.iter().find(|s| span_contains(s.def_span, offset))
    }

    /// Find a symbol by name (for go-to-definition).
    pub fn definition_of(&self, name: &str) -> Option<&Symbol> {
        self.symbols.iter().find(|s| s.name == name)
    }
}

fn collect_symbols(module: &Module, out: &mut Vec<Symbol>) {
    use certo_ast::decl::TypeBody;
    for sdecl in &module.decls {
        match &sdecl.node {
            Decl::Fn(f) => {
                let params: Vec<String> = f.params.iter()
                    .map(|p| format!("{}: {}", p.name.node, type_expr_hint(&p.ty.node)))
                    .collect();
                let ret = f.ret_ty.as_ref()
                    .map(|t| type_expr_hint(&t.node))
                    .unwrap_or_else(|| "_".to_string());
                let detail = format!("fn {}({}) -> {}", f.name.node, params.join(", "), ret);
                out.push(Symbol {
                    name:     f.name.node.clone(),
                    def_span: f.name.span,
                    kind:     SymbolKind::Function,
                    detail,
                });
                // Params
                for p in &f.params {
                    out.push(Symbol {
                        name:     p.name.node.clone(),
                        def_span: p.name.span,
                        kind:     SymbolKind::Param,
                        detail:   format!("param {}: {}", p.name.node, type_expr_hint(&p.ty.node)),
                    });
                }
            }
            Decl::Val(v) => {
                let ty_hint = v.ty.as_ref()
                    .map(|t| type_expr_hint(&t.node))
                    .unwrap_or_else(|| "_".to_string());
                let name = match &v.pattern.node {
                    certo_ast::pattern::Pattern::Ident { name, .. } => name.node.clone(),
                    _ => "<pattern>".to_string(),
                };
                out.push(Symbol {
                    name:     name.clone(),
                    def_span: v.pattern.span,
                    kind:     SymbolKind::Const,
                    detail:   format!("val {}: {}", name, ty_hint),
                });
            }
            Decl::Type(t) => {
                let kind_str = match &t.body {
                    TypeBody::Record(_) => "record",
                    TypeBody::Sum(_)    => "enum",
                    TypeBody::Alias(_)  => "alias",
                };
                out.push(Symbol {
                    name:     t.name.node.clone(),
                    def_span: t.name.span,
                    kind:     SymbolKind::Type,
                    detail:   format!("type {} ({})", t.name.node, kind_str),
                });
            }
            _ => {}
        }
    }
}

fn type_expr_hint(te: &certo_ast::types::TypeExpr) -> String {
    use certo_ast::types::TypeExpr;
    match te {
        TypeExpr::Named { path, args, .. } => {
            let base = path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");
            if args.is_empty() {
                base
            } else {
                let a = args.iter().map(|a| type_expr_hint(&a.node)).collect::<Vec<_>>().join(", ");
                format!("{}<{}>", base, a)
            }
        }
        TypeExpr::Option { inner, .. }  => format!("{}?", type_expr_hint(&inner.node)),
        TypeExpr::Tuple  { elements, .. } => format!("({})", elements.iter().map(|e| type_expr_hint(&e.node)).collect::<Vec<_>>().join(", ")),
        TypeExpr::Fn     { params, ret, .. } => {
            let ps = params.iter().map(|p| type_expr_hint(&p.node)).collect::<Vec<_>>().join(", ");
            format!("({}) -> {}", ps, type_expr_hint(&ret.node))
        }
        TypeExpr::Record { fields, .. } => {
            let fs = fields.iter().map(|f| format!("{}: {}", f.name.node, type_expr_hint(&f.ty.node))).collect::<Vec<_>>().join(", ");
            format!("{{ {} }}", fs)
        }
        TypeExpr::Ptr    { inner, .. }  => format!("*{}", type_expr_hint(&inner.node)),
        TypeExpr::Param  { name, .. }   => name.node.clone(),
    }
}
