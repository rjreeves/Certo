# Certo Compiler — Validator Feature Implementation Guide

## Overview

This document describes the implementation of the `validator`, `constraint`, and `temporal` features in the Certo compiler. It is written as a phase-by-phase implementation guide, following the existing compiler pipeline stages defined in section 11.1 of the language specification.

Each phase is described with:

* What changes
* Where in the existing codebase the change lives
* Precise inputs and outputs
* Edge cases to handle
* Tests to write before coding (TDD order)

Implementation order follows the compiler pipeline:

1. Lexer
2. Parser + AST
3. Name Resolution
4. Type Checker
5. Code Generator

Do not skip phases. Each phase's output is the next phase's input. The type checker in particular depends on correct AST structure from the parser, and correct symbol bindings from name resolution.

---

## Phase 1 — Lexer

### What Changes

New keywords must be recognised as `Token::Keyword` rather than `Token::Identifier`. New punctuation or structural tokens if any.

### New Keywords

Add to the keyword table in `src/lexer/keywords.rs` (or equivalent):

```
"validator"     → Token::Keyword(Keyword::Validator)
"constraint"    → Token::Keyword(Keyword::Constraint)
"temporal"      → Token::Keyword(Keyword::Temporal)
"rule"          → Token::Keyword(Keyword::Rule)
"require"       → Token::Keyword(Keyword::Require)
"after"         → Token::Keyword(Keyword::After)
"overrides"     → Token::Keyword(Keyword::Overrides)
"priority"      → Token::Keyword(Keyword::Priority)
"trigger"       → Token::Keyword(Keyword::Trigger)
"context"       → Token::Keyword(Keyword::Context)
"errors"        → Token::Keyword(Keyword::Errors)
"loaded"        → Token::Keyword(Keyword::Loaded)   -- part of 'loaded by'
"ruleTest"      → Token::Keyword(Keyword::RuleTest)
"validatorTest" → Token::Keyword(Keyword::ValidatorTest)
```

Note: `by` is already a keyword or common identifier. `loaded by` is a two-token sequence — the parser handles the pairing, not the lexer. `not in` is similarly two tokens — already handled by the existing expression parser.

### Existing Keywords Already Present

These are already in the spec keyword list and should already be in the lexer. Verify they are present — the validator feature uses them:

```
"pub"     "for"     "else"    "when"
"and"     "or"      "not"     "in"
"on"      "import"  "module"
```

### Token Conflicts to Check

* `context` — check it does not clash with any existing identifier used as a soft keyword elsewhere in the language. If it does, treat it as a contextual keyword (only keyword inside validator body, identifier elsewhere).
* `rule` — same check. Likely safe as it is not a common identifier in business code, but verify.
* `trigger` — same check.

### Lexer Tests to Write First

```
// Each keyword tokenises correctly
lex("validator")  == [Token::Keyword(Keyword::Validator)]
lex("constraint") == [Token::Keyword(Keyword::Constraint)]
lex("temporal")   == [Token::Keyword(Keyword::Temporal)]
lex("rule")       == [Token::Keyword(Keyword::Rule)]
lex("require")    == [Token::Keyword(Keyword::Require)]
lex("after")      == [Token::Keyword(Keyword::After)]
lex("overrides")  == [Token::Keyword(Keyword::Overrides)]
lex("priority")   == [Token::Keyword(Keyword::Priority)]
lex("trigger")    == [Token::Keyword(Keyword::Trigger)]
lex("context")    == [Token::Keyword(Keyword::Context)]
lex("errors")     == [Token::Keyword(Keyword::Errors)]
lex("loaded")     == [Token::Keyword(Keyword::Loaded)]

// Keywords do not swallow adjacent identifiers
lex("validator_name") == [Token::Identifier("validator_name")]
lex("rule_id")        == [Token::Identifier("rule_id")]
lex("context_data")   == [Token::Identifier("context_data")]

// Keywords inside strings are not tokenised as keywords
lex("\"validator\"")  == [Token::StringLiteral("validator")]
```

---

## Phase 2 — Parser and AST

### What Changes

New AST node types for all new declarations. New grammar productions that consume the new tokens and produce those nodes. Integration into the top-level `declaration` production.

### New AST Nodes

Add to `src/ast/declarations.rs` (or equivalent):

```
// Top-level declarations
ConstraintDecl {
    span:   Span,
    pub:    bool,
    name:   Ident,
    body:   Expr,      // boolean expression — deferred type check
}

TemporalDecl {
    span:   Span,
    pub:    bool,
    name:   Ident,
    body:   Expr,      // must resolve to Duration — checked in type phase
}

ValidatorDecl {
    span:    Span,
    pub:     bool,
    name:    Ident,
    entity:  TypeRef,  // the 'for' type
    errors:  TypeRef,  // the 'errors' type
    trigger: Option<TriggerDecl>,
    context: Vec<ContextField>,
    rules:   Vec<RuleDecl>,
}

// Supporting nodes
TriggerDecl {
    span:      Span,
    op:        TriggerOp,         // Insert | Update
    condition: Option<TriggerCondition>,
}

TriggerOp {
    Insert,
    Update,
}

TriggerCondition {
    span:  Span,
    field: Ident,
    value: Expr,  // enum variant or literal
}

ContextField {
    span:      Span,
    name:      Ident,
    type_ref:  TypeRef,
    loaded_by: Option<Expr>,  // db expression for auto-loading
}

RuleDecl {
    span:      Span,
    name:      Ident,
    after:     Vec<Ident>,    // named prerequisite rules
    overrides: Option<Ident>, // named rule this suspends
    priority:  Option<i64>,   // default 0
    require:   Expr,          // boolean condition
    else_:     Expr,          // must produce errors type — Err(ErrorType.Variant)
}

// Test blocks
RuleTestDecl {
    span:      Span,
    validator: QualifiedIdent, // OrderSubmit.customer_active
    label:     StringLiteral,
    entity:    Expr,           // record literal
    context:   Expr,           // record literal
    expect:    TestExpectation,
}

ValidatorTestDecl {
    span:      Span,
    validator: Ident,
    label:     StringLiteral,
    entity:    Expr,
    context:   Expr,
    expect:    TestExpectation,
}

TestExpectation {
    Pass,
    Fail { with: Option<Expr> },  // optional specific error variant
}
```

### Grammar Productions

Add to `src/parser/grammar.rs` (or equivalent). Written in EBNF matching the style of Appendix A:

```ebnf
declaration ::= ...existing...
              | constraint_decl
              | temporal_decl
              | validator_decl
              | rule_test_decl
              | validator_test_decl

-- Constraint declaration
constraint_decl ::= "pub"? "constraint" IDENT "=" expr

-- Temporal declaration
temporal_decl ::= "pub"? "temporal" IDENT "=" expr

-- Validator declaration
validator_decl ::= "pub"? "validator" IDENT
                   "for" type_ref
                   "errors" type_ref
                   trigger_ann?
                   "{" context_block? rule_decl* "}"

trigger_ann ::= "trigger" "on" trigger_op trigger_condition?

trigger_op ::= "Insert" | "Update"

trigger_condition ::= "when" IDENT "==" expr
                    | "when" IDENT "!=" expr
                    | "when" IDENT "!=" "OLD" "." IDENT

context_block ::= "context" "{" context_field* "}"

context_field ::= IDENT ":" type_ref ("loaded" "by" expr)?

rule_decl ::= "rule" IDENT "{" rule_body "}"

rule_body ::= rule_after*
              rule_overrides?
              rule_priority?
              "require" expr
              "else" expr

rule_after     ::= "after" IDENT
rule_overrides ::= "overrides" IDENT
rule_priority  ::= "priority" INT_LITERAL

-- Test declarations
rule_test_decl ::= "ruleTest" qualified_ident STRING_LITERAL "{"
                   "entity"  ":" expr
                   "context" ":" expr
                   "expect"  ":" test_expectation
                   "}"

validator_test_decl ::= "validatorTest" IDENT STRING_LITERAL "{"
                        "entity"  ":" expr
                        "context" ":" expr
                        "expect"  ":" test_expectation
                        "}"

test_expectation ::= "pass"
                   | "fail"
                   | "fail" "with" expr

qualified_ident ::= IDENT ("." IDENT)*
```

### Expression Grammar Extensions

The `require` clause uses the existing boolean expression grammar. Two additions needed:

```ebnf
-- Temporal age access — new postfix on field access chain
postfix_expr ::= ...existing...
               | postfix_expr "." "age"    -- new: only valid on Timestamp fields

-- 'not in' — two-token sequence, treat as single operator
-- Already handled if 'in' is a binary operator and 'not' is prefix
-- Verify the parser produces:
--   not (x in [...])   rather than   (not x) in [...]
-- The former is correct.
```

### Parser Implementation Notes

**Rule body ordering** — `after`, `overrides`, and `priority` can appear in any order before `require`. The parser should accept any ordering and normalise into the AST node. Do not require a specific order.

**`loaded by` two-token sequence** — parse `"loaded"` then expect `"by"` immediately after. If `"by"` is not present, emit parse error:

```
error: expected `by` after `loaded`
```

**`trigger on Update when OLD.status != NEW.status`** — `OLD` and `NEW` are contextual identifiers in trigger conditions, not keywords. Parse them as identifiers and validate in the type checker phase.

**Error recovery** — if a rule body is malformed, try to recover at the next `rule` keyword or closing `}` so the parser can report multiple errors in one pass.

### Parser Tests to Write First

```
// Minimal validator parses
parse("validator V for Order errors OrderError {
    rule r { require true else Err(OrderError.X) }
}")

// Full validator with all features parses
parse("pub validator OrderSubmit for Order
    errors OrderError
    trigger on Update when status == Submitted
{
    context {
        customer: Customer  loaded by db.customers.find(order.customerId)
        user:     User      loaded by db.users.find(currentUserId())
    }
    rule customer_active {
        after some_rule
        overrides other_rule
        priority 100
        require customer.status == Active
        else Err(OrderError.CustomerNotActive)
    }
}")

// Constraint declaration parses
parse("pub constraint UserIsAdmin = user.role == Admin")
parse("constraint WithinCredit = order.total <= customer.availableCredit")

// Temporal declaration parses
parse("pub temporal VoidWindow = Duration.days(30)")
parse("temporal GracePeriod = Duration.hours(48)")

// Rule with multiple afters parses
parse("rule r {
    after a
    after b
    after c
    require true
    else Err(E.X)
}")

// Trigger variants parse
parse("validator V for T errors E { trigger on Insert { ... } }")
parse("validator V for T errors E { trigger on Update { ... } }")
parse("validator V for T errors E { trigger on Update when status == Submitted { ... } }")
parse("validator V for T errors E { trigger on Update when status != OLD.status { ... } }")

// Test blocks parse
parse("ruleTest OrderSubmit.customer_active \"label\" {
    entity:  Order { id: uuid\"...\" }
    context: { customer: defaultCustomer, user: defaultUser }
    expect:  pass
}")

parse("validatorTest OrderSubmit \"label\" {
    entity:  validOrder
    context: { customer: activeCustomer, user: salesUser }
    expect:  fail with OrderError.CustomerNotActive
}")

// Parse errors reported correctly
parse_error("validator V for Order {")          // missing 'errors'
parse_error("rule r { require true }")          // missing 'else'
parse_error("context { x: T loaded }")          // missing 'by'
parse_error("validator V errors E for Order")   // wrong order
```

---

## Phase 3 — Name Resolution

### What Changes

New declaration kinds must be registered in the module symbol table. `pub` visibility must be handled. References to constraints and temporals inside validator bodies must be resolved to their declaration sites. References within `after` and `overrides` must be resolved to rule declarations within the same validator.

### Symbol Table Additions

Add to `src/resolve/symbol_table.rs` (or equivalent):

```
// New symbol kinds
Symbol::Constraint {
    decl:   ConstraintDecl,
    module: ModuleId,
    pub:    bool,
}

Symbol::Temporal {
    decl:   TemporalDecl,
    module: ModuleId,
    pub:    bool,
}

Symbol::Validator {
    decl:   ValidatorDecl,
    module: ModuleId,
    pub:    bool,
}
```

Rules are not top-level symbols — they live inside their validator's symbol entry and are resolved locally, not globally.

### Resolution Pass

**Constraint declarations** — register in module symbol table. Mark as `pub` if declared `pub`. Body expression is stored as-is — do NOT attempt to resolve field references at this stage. Body resolution is deferred to the type checker, which has context.

**Temporal declarations** — register in module symbol table. Body expression IS resolved at this stage — it references only stdlib functions (`Duration.days`, `Duration.hours`) which are already resolvable. Report error E0100 if `Duration` module not in scope.

**Validator declarations** — register in module symbol table. Resolve `for` type reference against known types. Resolve `errors` type reference against known types. Store rule declarations without resolving their bodies — deferred to type checker.

**Context fields** — resolve the declared type of each context field against known types. Store `loaded by` expression without resolving — deferred to type checker which has DB schema access.

**`after` references** — build a local rule name map for each validator. Resolve each `after` identifier against that map. Report E0701 if the named rule does not exist in this validator.

**`overrides` references** — resolve against the same local rule name map. Report E0702 if the named rule does not exist in this validator.

**Cycle detection** — after resolving all `after` references within a validator, build the dependency graph and check for cycles using DFS. Report E0700 if a cycle is found. Include the full cycle path in the error message:

```
error[E0700]: cycle detected in rule dependency graph
  = note: cycle: rule_a → rule_b → rule_a
```

**Import resolution** — constraints and temporals marked `pub` are exported from their module. When another module imports them, resolve the import to the constraint/temporal symbol. The imported name is then available for use in `require` clauses within that module's validators.

### Resolution Tests to Write First

```
// Constraint registered in symbol table after resolution
resolve("pub constraint UserIsAdmin = user.role == Admin")
// → symbol table contains Constraint { name: "UserIsAdmin", pub: true }

// Temporal body resolved — Duration.days available
resolve("pub temporal VoidWindow = Duration.days(30)")
// → symbol table contains Temporal { name: "VoidWindow", pub: true }

// Validator registered — for/errors types resolved
resolve("validator V for Order errors OrderError { ... }")
// → symbol table contains Validator { name: "V", entity: Order, errors: OrderError }

// after reference to existing rule resolves
resolve_validator("
    rule a { require true else Err(E.X) }
    rule b { after a  require true else Err(E.X) }
")
// → b.after[0] resolved to rule 'a'

// after reference to missing rule reports E0701
resolve_error("rule b { after nonexistent require true else Err(E.X) }", E0701)

// overrides reference to existing rule resolves
resolve_validator("
    rule a { require true else Err(E.X) }
    rule b { overrides a  require true else Err(E.X) }
")
// → b.overrides resolved to rule 'a'

// overrides reference to missing rule reports E0702
resolve_error("rule b { overrides nonexistent require true else Err(E.X) }", E0702)

// Cycle detected and reported as E0700
resolve_error("
    rule a { after b  require true else Err(E.X) }
    rule b { after a  require true else Err(E.X) }
", E0700)

// pub constraint importable from other module
resolve_import("import MyModule.{ UserIsAdmin }")
// → UserIsAdmin available in importing module's scope

// non-pub constraint not importable
resolve_error("import MyModule.{ PrivateConstraint }", E0101)
```

---

## Phase 4 — Type Checker

This is the most substantial phase. The type checker has access to the live DB schema, resolved types, and the full symbol table. It performs all the checks that were deferred from earlier phases.

### What Changes

New type checking logic for each new AST node. DB schema verification for `context` field types and `loaded by` expressions. Constraint body resolution using validator context as the type environment. Temporal body verification against `Duration`. Error type verification for `else` branches. Trigger field and value verification.

### Temporal Type Checking

For each `TemporalDecl`:

1. Type-check the body expression.
2. Verify the result type is `Duration`.
3. If not `Duration`, report E0708:

```
error[E0708]: temporal declaration does not resolve to Duration
  = found: Int
  = help: use Duration.days(n), Duration.hours(n), etc.
```

### Constraint Type Checking

Constraints are checked lazily — at the point of use inside a validator `require` clause, not at the point of declaration.

When a `require` clause references a constraint name:

1. Look up the constraint in the symbol table.
2. Retrieve its body expression (stored as unresolved AST).
3. Type-check the body expression in the current type environment:
   * Primary entity fields in scope (the `for` type)
   * Context fields in scope (from the validator's `context` block)
4. Verify the result type is `Bool`.
5. If any field referenced in the body is not in scope, report E0704:

```
error[E0704]: constraint references field `user` not in validator context
  = note: UserIsAdmin references `user.role`
  = note: `user` is not declared in this validator's context block
  = help: add `user: User` to the context block
```

### Validator Type Checking

For each `ValidatorDecl`:

**Entity type** — verify the `for` type exists and is a record type (not a primitive, not a sum type). Report E0200 if not a record type.

**Errors type** — verify the `errors` type exists and is a sum type. Report E0200 if not a sum type. All `else` branches in rules must produce this type.

**Context fields** — for each `ContextField`:

1. Verify the declared type exists.
2. If `loaded by` is present:
   a. Type-check the `loaded by` expression.
   b. Verify the expression result type matches the declared field type.
   c. Verify column references in the expression against DB schema.
   d. Report E0705 on type mismatch, E0402 on schema mismatch.

**Rules — `require` clause** — type-check in the validator's type environment (primary entity + context fields). Verify result is `Bool`. Inline any referenced constraints and type-check them in this context. Verify `.age` is only used on `Timestamp` or `Timestamp?` fields (E0709).

**Rules — `else` clause** — verify the expression produces a value of the declared `errors` type. The expression must be of the form `Err(ErrorType.Variant(...))`. Report E0703 on type mismatch.

**`after` ordering** — dependency graph was built in name resolution. Verify in type checker that:

* Rules used in `after` were successfully type-checked.
* If a prerequisite rule has a type error, skip dependent rules (avoid cascading errors).

**`overrides` relationship** — verify that the overriding rule's `require` type-checks correctly. Emit W0101 if the overriding rule's condition can be statically determined to always pass — the overridden rule would be permanently unreachable.

**`priority` conflicts** — after type-checking all rules, scan for pairs of rules where one overrides the other without explicit `priority` declarations. Emit W0100 for each such pair.

### Trigger Type Checking

For each `TriggerDecl`:

1. Verify `trigger on Insert` or `trigger on Update`.
2. If `when` condition present:
   a. Verify the named field exists on the entity.
   b. Verify the value is a valid variant of the field's type (if enum) or a compatible literal (if scalar).
   c. Report E0706 if field not found, E0707 if value not valid.
3. `OLD` in trigger conditions refers to the pre-update row. `OLD.field` — verify `field` exists on the entity. Same check as for `NEW.field`.

### `.age` Property Type Checking

`.age` is a computed property. Add to the type checker's postfix expression handling:

```
// When resolving postfix: expr.age
if postfix == "age" {
    let base_type = type_of(expr)
    match base_type {
        Timestamp  => Ok(Duration)
        Timestamp? => Ok(Duration)   // returns Duration.max for None
        other      => Err(E0709 { found: other })
    }
}
```

### Type Checker Tests to Write First

```
// Temporal type checks against Duration
typecheck_ok("pub temporal VoidWindow = Duration.days(30)")
typecheck_ok("pub temporal Window = Duration.hours(48)")
typecheck_err("pub temporal Bad = 30", E0708)          // Int not Duration
typecheck_err("pub temporal Bad = \"30 days\"", E0708) // Text not Duration

// Constraint type-checks in validator context
typecheck_ok("
    pub constraint UserIsAdmin = user.role == Admin
    validator V for Order errors OE {
        context { user: User }
        rule r { require UserIsAdmin else Err(OE.X) }
    }
")

// Constraint field not in context → E0704
typecheck_err("
    pub constraint UserIsAdmin = user.role == Admin
    validator V for Order errors OE {
        // no user in context
        rule r { require UserIsAdmin else Err(OE.X) }
    }
", E0704)

// else branch wrong type → E0703
typecheck_err("
    validator V for Order errors OrderError {
        rule r {
            require true
            else Err(BillingError.X)    // wrong type
        }
    }
", E0703)

// loaded by type mismatch → E0705
typecheck_err("
    validator V for Order errors OE {
        context {
            customer: Customer loaded by db.users.find(order.customerId)
            // db.users.find returns User, not Customer
        }
        rule r { require true else Err(OE.X) }
    }
", E0705)

// .age on Timestamp field → ok
typecheck_ok("
    validator V for Invoice errors IE {
        rule r {
            require invoice.createdAt.age < VoidWindow
            else Err(IE.X)
        }
    }
")

// .age on non-Timestamp field → E0709
typecheck_err("
    validator V for Invoice errors IE {
        rule r {
            require invoice.status.age < VoidWindow   // status is not Timestamp
            else Err(IE.X)
        }
    }
", E0709)

// Trigger field exists → ok
typecheck_ok("
    validator V for Order errors OE
        trigger on Update when status == Submitted { ... }
")

// Trigger field not on entity → E0706
typecheck_err("
    validator V for Order errors OE
        trigger on Update when nonexistent == Submitted { ... }
", E0706)

// Trigger value not valid variant → E0707
typecheck_err("
    validator V for Order errors OE
        trigger on Update when status == NotARealVariant { ... }
", E0707)

// W0101 — unreachable rule (always-true override)
typecheck_warn("
    validator V for Order errors OE {
        rule base { require order.total > 0  else Err(OE.X) }
        rule override_ {
            overrides base
            require true    // always true — base is permanently shadowed
            else Err(OE.X)
        }
    }
", W0101)
```

---

## Phase 5 — Code Generator

### What Changes

New code generation for validators produces:

* Two functions per validator: `validate` and `validateAll`
* One additional function when all context fields have `loaded by`: `validateWithDb`
* Trigger SQL when `trigger` is declared
* Test scaffolding for `ruleTest` and `validatorTest` blocks

The generator emits Certo's existing IR (HIR or MIR depending on pipeline stage) for the validator functions. Trigger SQL is emitted as a side-channel output alongside the binary.

### Generated Function Signatures

For validator `OrderSubmit for Order errors OrderError` with context `{ customer: Customer, user: User }`:

```certo
// Generated — do not edit

type OrderSubmitContext = {
    customer: Customer,
    user:     User
}

fn OrderSubmit.validate(
    order:   Order,
    context: OrderSubmitContext
): Result<Unit, OrderError>

fn OrderSubmit.validateAll(
    order:   Order,
    context: OrderSubmitContext
): List<OrderError>

// Only generated when all context fields have 'loaded by'
async fn OrderSubmit.validateWithDb(
    order: Order
): Result<Unit, OrderError>
```

### `validate` — Fail Fast Generation

For each rule in topological order (after dependency graph):

```
1. If rule has `overrides`:
     emit: if overriding_rule_condition(entity, context) { skip this rule }

2. If rule has `after`:
     emit: if not all(prerequisite_passed_flags) { skip this rule }

3. Emit condition check:
     if not require_expr { return Err(else_expr) }

4. If this rule is a prerequisite for others:
     emit: let rule_name_passed = require_expr
```

Constraint references in `require_expr` are inlined at this stage — the constraint body replaces the constraint name in the emitted IR. No runtime constraint lookup.

### `validateAll` — Collect All Generation

Same structure as `validate` but:

* Instead of `return Err(...)`, emit `violations.append(else_expr)`
* At the end, return `violations`
* Prerequisites still gate dependent rules — if `customer_active` fails, `within_credit_limit` is still skipped in `validateAll` mode

### `validateWithDb` — Context Loading Generation

Only generated when all context fields have `loaded by`.

```
async fn OrderSubmit.validateWithDb(order: Order): Result<Unit, OrderError> =
    db.transaction {
        let customer = <loaded_by_expr_for_customer>?
        let user     = <loaded_by_expr_for_user>?
        let context  = OrderSubmitContext { customer, user }
        OrderSubmit.validate(order, context)?
        Ok(())
    }
```

If any `loaded by` expression fails (returns `Err`), the error is propagated with `?`. The `loaded by` error type must be compatible with the validator's `errors` type, or a `mapErr` is emitted automatically.

### Trigger SQL Generation

When a validator declares `trigger`, emit SQL as a side-channel output to `dist/triggers/<validator_name>.sql`:

```sql
-- Generated by certo build
-- Validator: OrderSubmit
-- Entity:    orders
-- Trigger:   BEFORE UPDATE when status = 'SUBMITTED'

CREATE OR REPLACE FUNCTION trg_validate_orders_submit()
RETURNS TRIGGER AS $$
DECLARE
    v_context JSONB;
BEGIN
    -- Only fire when status transitions to SUBMITTED
    IF NEW.status != 'SUBMITTED' OR OLD.status = 'SUBMITTED' THEN
        RETURN NEW;
    END IF;

    v_context := current_rule_context();

    -- Rule: customer_active
    IF (v_context->>'customer_status') != 'ACTIVE' THEN
        RAISE EXCEPTION 'CUSTOMER_NOT_ACTIVE|...' USING ERRCODE = 'P0001';
    END IF;

    -- ... remaining rules ...

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_validate_orders_submit ON orders;
CREATE TRIGGER trg_validate_orders_submit
    BEFORE UPDATE ON orders
    FOR EACH ROW
    EXECUTE FUNCTION trg_validate_orders_submit();
```

When targeting WASM (`certo build --target wasm`), emit WASM module instead of SQL. The trigger wiring SQL is still emitted but references the WASM function rather than a PL/pgSQL body.

### Test Block Generation

`ruleTest` blocks compile to test functions in the test runner:

```certo
// ruleTest OrderSubmit.customer_active "passes for active customer" { ... }
// compiles to:

test "OrderSubmit.customer_active: passes for active customer" {
    let entity  = <entity_expr>
    let context = <context_expr>
    let result  = OrderSubmit.__check_customer_active(entity, context)
    expect(result).toBe(true)
}
```

Each rule generates a `__check_<rule_name>` private function that returns `Bool` — the raw condition result. This is what `ruleTest` calls. The double underscore prefix marks it as generated/internal.

`validatorTest` blocks compile to:

```certo
test "OrderSubmit: passes for valid order" {
    let entity  = <entity_expr>
    let context = <context_expr>
    let result  = OrderSubmit.validate(entity, context)
    expect(result).toBeOk()
}

test "OrderSubmit: fails when credit limit exceeded" {
    let result  = OrderSubmit.validate(entity, context)
    expect(result).toBeErr(OrderError.CreditLimitExceeded)
}
```

### Code Generator Tests to Write First

```
// validate function generated with correct signature
codegen_has_fn("OrderSubmit.validate",
    params: [(Order, "order"), (OrderSubmitContext, "context")],
    returns: Result<Unit, OrderError>
)

// validateAll function generated
codegen_has_fn("OrderSubmit.validateAll",
    params: [(Order, "order"), (OrderSubmitContext, "context")],
    returns: List<OrderError>
)

// validateWithDb generated when all context fields have loaded by
codegen_has_fn("OrderSubmit.validateWithDb",
    params: [(Order, "order")],
    returns: Result<Unit, OrderError>
)

// validateWithDb NOT generated when context field lacks loaded by
codegen_no_fn("V.validateWithDb")

// override rule suppresses target rule in generated code
codegen_contains("OrderSubmit.validate",
    "if Check_CreditLimitAdminOverride(order, context) { /* skip within_credit_limit */ }"
)

// after dependency gates rule evaluation
codegen_contains("OrderSubmit.validate",
    "if customer_active_passed { /* evaluate within_credit_limit */ }"
)

// Constraint inlined — no runtime lookup
codegen_not_contains("OrderSubmit.validate", "constraint_lookup")
codegen_not_contains("OrderSubmit.validate", "UserIsAdmin_ref")

// Trigger SQL emitted to dist/triggers/
codegen_file_exists("dist/triggers/order_submit.sql")
codegen_contains_sql("dist/triggers/order_submit.sql",
    "BEFORE UPDATE ON orders"
)

// ruleTest generates __check_ private function
codegen_has_fn("OrderSubmit.__check_customer_active",
    returns: Bool
)
```

---

## Phase 6 — `certo generate` Command

The `certo generate` command gains a new subcommand for consuming YAML rule definitions:

```
certo generate validators \
    --prepared-items path/to/prepared-items.yaml \
    --entities       path/to/entities.yaml \
    --rules          path/to/rules.yaml \
    --output         src/validators/
```

### What It Produces

```
src/validators/
    constraints.cto
    temporals.cto
    order-validators.cto
    invoice-validators.cto
    payment-validators.cto
    order-line-validators.cto
    ...
```

### Implementation

The generator reads the three YAML files, resolves cross-references (same logic as the C# DefinitionResolver), and emits `.cto` source using the section 16 syntax.

It is implemented in Certo itself — `src/tools/generate/validators.cto`. This is a `[io]` effect function that reads YAML and writes `.cto` files.

YAML parsing uses a Certo YAML library (or C FFI to an existing C YAML parser if the stdlib does not yet have one).

### Generator Output Contract

The generator must produce `.cto` files that:

1. Pass `certo check` with no errors
2. Pass `certo build` against the database schema produced by the Prisma generator from the same YAML input
3. Are idempotent — running the generator twice produces identical output

Idempotency is important because developers may run `certo generate` after editing YAML, and the output should not produce spurious diffs.

---

## Implementation Checklist

### Phase 1 — Lexer

- [ ] Add new keywords to keyword table
- [ ] Write lexer tests
- [ ] Verify no conflicts with existing identifiers
- [ ] All lexer tests pass

### Phase 2 — Parser

- [ ] Add new AST node types
- [ ] Add grammar productions
- [ ] Write parser tests
- [ ] Verify error recovery on malformed input
- [ ] All parser tests pass

### Phase 3 — Name Resolution

- [ ] Add new symbol kinds
- [ ] Register constraint/temporal/validator declarations
- [ ] Resolve after/overrides references within validator scope
- [ ] Detect and report dependency cycles
- [ ] Handle pub visibility and imports
- [ ] Write resolution tests
- [ ] All resolution tests pass

### Phase 4 — Type Checker

- [ ] Temporal body → Duration verification
- [ ] Constraint deferred resolution in validator context
- [ ] Validator entity/errors type verification
- [ ] Context field type verification
- [ ] loaded by expression type checking against DB schema
- [ ] require clause type checking in validator context
- [ ] else clause type matching against errors type
- [ ] .age postfix property type rule
- [ ] Trigger field/value verification
- [ ] W0100 conflict detection
- [ ] W0101 unreachable rule detection
- [ ] Write type checker tests
- [ ] All type checker tests pass

### Phase 5 — Code Generator

- [ ] validate function generation
- [ ] validateAll function generation
- [ ] validateWithDb conditional generation
- [ ] Constraint inlining
- [ ] Override rule suppression logic
- [ ] after dependency gating logic
- [ ] Trigger SQL emission
- [ ] ruleTest __check function generation
- [ ] validatorTest scaffolding generation
- [ ] Write codegen tests
- [ ] All codegen tests pass

### Phase 6 — certo generate command

- [ ] YAML parsing
- [ ] Cross-reference resolution
- [ ] constraints.cto emission
- [ ] temporals.cto emission
- [ ] Per-entity validator file emission
- [ ] Idempotency verification
- [ ] Integration test: YAML → generate → build → test all pass

### Final Integration Test

```
// Full pipeline from YAML to running system
certo generate validators --prepared-items ... --entities ... --rules ...
certo check
certo build
certo db migrate
certo test
```

All commands must exit 0. No errors, no warnings (except W0002 on any Float usage if present in test fixtures).

---

## Appendix — Error Code Summary

| Code  | Phase      | Description                                              |
|-------|------------|----------------------------------------------------------|
| E0700 | Resolution | Cycle in `after` dependency graph                        |
| E0701 | Resolution | `after` references unknown rule in this validator        |
| E0702 | Resolution | `overrides` references unknown rule in this validator    |
| E0703 | Type       | `else` branch type does not match declared `errors` type |
| E0704 | Type       | Constraint field not in scope — not in `context` block   |
| E0705 | Type       | `loaded by` expression type mismatch                     |
| E0706 | Type       | `trigger` field does not exist on entity                 |
| E0707 | Type       | `trigger` value not a valid variant of field type        |
| E0708 | Type       | `temporal` body does not resolve to `Duration`           |
| E0709 | Type       | `.age` used on non-`Timestamp` field                     |
| W0100 | Type       | Rule conflict without explicit `priority` or `overrides` |
| W0101 | Type       | Unreachable rule — permanently shadowed by `overrides`   |
| W0102 | Codegen    | `validateWithDb` unavailable — missing `loaded by`       |
