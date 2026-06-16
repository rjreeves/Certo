# Rule Generator — Schema Definitions

Three YAML definition files form the source of truth for the entire generation
pipeline. Everything below — DB schema, validators, guards, tests, docs — is
derived from these files.

---

## File Overview

```
prepared-items.schema.yaml    ← shared vocabulary (types, enums, temporal, constraints)
entities.schema.yaml          ← domain entities (fields, relations, indexes)
rules.schema.yaml             ← business rules (conditions, errors, generation targets)
```

---

## Dependency Order

```
prepared-items.schema.yaml
        ↓ referenced by
entities.schema.yaml
        ↓ referenced by
rules.schema.yaml
```

Entities reference types and enums from prepared items.
Rules reference entities, and also types/enums/temporal/constraints from prepared items.
Prepared items have no dependencies.

---

## What Gets Generated

```
All three files
      ↓
┌─────────────────────────────────────────────┐
│             Generation Targets              │
├──────────────────┬──────────────────────────┤
│ From entities    │ From rules               │
│                  │                          │
│ Prisma schema    │ C# validator classes     │
│ Postgres tables  │ PL/pgSQL functions       │
│ Postgres domains │ API middleware guards    │
│ Postgres enums   │ State machines           │
│ Foreign keys     │ Test cases               │
│ Indexes          │ Documentation            │
│ TypeScript types │                          │
└──────────────────┴──────────────────────────┘
```

---

## Key Concepts

### Prepared Items
The vocabulary layer. Define once, reference everywhere.

- **Types** — named scalar types: `email`, `money`, `percentage`
- **Enums** — named enumerations with optional state transitions: `order_status`
- **Temporal** — named time windows: `standard_void_window` (30 days)
- **Constraints** — named reusable condition fragments: `within_credit_limit`

### Entities
Domain objects with typed fields and relationships.

- Fields reference prepared types and enums by id
- Reference fields generate foreign keys and relations
- `audit: true` adds `createdAt`, `updatedAt`, `createdBy`
- `soft_delete: true` adds `deletedAt` instead of hard deletes

### Rules
Behavioural constraints on entity operations.

- Conditions are recursive AND/OR/NOT trees
- Leaf conditions reference entity fields via dot notation
- Named constraints and temporals from prepared items keep rules DRY
- `depends_on` chains rules — dependent rules only evaluate if prerequisites pass
- `overrides` suspends another rule when this rule's condition passes
- `priority` resolves conflicts — higher value wins
- `targets` controls what each generator emits for this rule

---

## Condition Dot Notation

Field paths use dot notation to traverse entity relationships:

```yaml
field: user.verified          # field on related entity
field: product.inventoryCount # field on related entity
field: order.total            # field on current entity
field: discount.amount        # nested field
```

Value can be a literal or another field reference:

```yaml
value: true                   # literal boolean
value: 0                      # literal number
value: PAID                   # literal enum value
value: user.creditLimit       # field reference — compares two fields
value: product.margin         # field reference
```

---

## Generator CLI

```bash
dotnet run -- generate \
  --rules rules.schema.yaml \
  --entities entities.schema.yaml \
  --prepared-items prepared-items.schema.yaml \
  --output ./generated \
  --targets csharp plpgsql prisma
```

---

## Validation Rules

The generator lints definitions before generating:

- Every type referenced in an entity must exist in prepared items
- Every enum referenced in an entity must exist in prepared items
- Every entity referenced in a rule must exist in entity definitions
- Every constraint referenced in a rule must exist in prepared items
- Every temporal referenced in a rule must exist in prepared items
- Every `depends_on` rule id must exist in rule definitions
- Every `overrides` rule id must exist in rule definitions
- Condition operators must appear in the prepared items operator list
- Enum transition maps must only reference values declared in that enum
