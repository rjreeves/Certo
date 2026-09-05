# 16. Validators

Validators are named, compiler-verified declarations that enforce business
rules on entity operations. A validator groups related rules for a specific
entity and trigger, produces typed errors, and optionally installs itself
as a database trigger.

Validators are first-class language constructs — not library functions,
not annotations, not middleware. They live alongside your type declarations,
carry the same visibility rules, and are verified by the same compiler that
verifies your schema.

The core promise: if a validator compiles, every rule it contains is
structurally sound. Field references are verified against your entity types
and database schema. Error types are verified against your declared error
union. Dependency relationships between rules are verified to be acyclic.
None of these are runtime checks.

---

## 16.1 Declaration Syntax

A validator declaration names the entity it guards, the error type it
produces, and a body containing context declarations and rule declarations.

```certo
validator <Name> for <EntityType>
    errors <ErrorType>
    [trigger on Insert | Update when <field> == <value>]
{
    context {
        <name>: <Type> [loaded by <expr>]
        ...
    }

    rule <name> {
        [after  <rule_name>]*
        [overrides <rule_name>]
        [priority <Int>]
        require <condition>
        else <ErrorType.Variant(...)>
    }

    ...
}
```

Minimal example:

```certo
validator InvoiceIssue for Invoice errors InvoiceError {
    rule invoice_is_draft {
        require invoice.status == Draft
        else InvoiceError.NotInDraftStatus(invoice.status)
    }
}
```

Full example:

```certo
validator OrderSubmit for Order
    errors OrderError
    trigger on Update when status == Submitted
{
    context {
        customer: Customer  loaded by db.customers.find(order.customerId)
        user:     User      loaded by db.users.find(currentUserId())
    }

    rule customer_active {
        require CustomerIsActive
        else OrderError.CustomerNotActive
    }

    rule has_lines {
        require order.lineCount > 0
        else OrderError.NoOrderLines
    }

    rule has_shipping_address {
        require order.shippingAddressId.isSome()
            or  order.linesAllDigital
        else OrderError.NoShippingAddress
    }

    rule within_credit_limit {
        after customer_active
        require WithinCredit
        else OrderError.CreditLimitExceeded(
            customer.availableCredit,
            order.total
        )
    }

    rule credit_limit_admin_override {
        overrides within_credit_limit
        priority 110
        require UserIsAdmin
        else OrderError.NotAuthorised
    }
}
```

---

## 16.2 The `context` Block

The context block declares the related entities a validator needs to
evaluate its rules. The primary entity — the one named in `for <EntityType>`
— is always in scope as a lowercase binding of its type name. Related
entities are declared explicitly.

```certo
validator OrderSubmit for Order errors OrderError {

    // `order` is in scope automatically — the primary entity
    // Declare everything else the rules need:
    context {
        customer: Customer
        user:     User
    }

    rule has_lines {
        require order.lineCount > 0    // primary entity — always available
        else OrderError.NoOrderLines
    }

    rule customer_active {
        require customer.status == Active    // from context
        else OrderError.CustomerNotActive
    }
}
```

### Context Loading

Context fields can declare their own database loading expression using
`loaded by`. The expression is verified against the DB schema at compile
time — if `order.customerId` does not exist or `db.customers.find` does
not return `Customer`, the build fails.

```certo
context {
    customer: Customer  loaded by db.customers.find(order.customerId)
    user:     User      loaded by db.users.find(currentUserId())
}
```

When all context fields carry `loaded by` expressions, the compiler
generates a `validateWithDb` variant that loads context automatically
inside the current transaction. See section 16.6.

### Context Scope

Context fields are only in scope within the validator that declares them.
They are not accessible from outside the validator. Named constraints that
reference context fields (see section 16.9) are resolved against the
enclosing validator's context at the point of use — not at the point of
constraint declaration.

---

## 16.3 Rules

A rule is a named condition within a validator. It consists of a `require`
clause containing a boolean condition, and an `else` clause producing a
typed error value.

```certo
rule payment_method_present {
    require user.paymentMethod.isSome()
        or  order.paymentMethod == Prepaid
    else OrderError.NoPaymentMethod
}
```

Rules are evaluated in declaration order, subject to `after` and `overrides`
relationships. All rules in a validator share the same entity binding and
context fields.

### Rule Names

Rule names are identifiers in snake_case. They must be unique within a
validator. They are used by `after` and `overrides` clauses to express
relationships between rules. They appear in compiler error messages and
generated documentation.

### The `require` Clause

The `require` clause contains a boolean expression evaluated against the
primary entity and the context fields. The full set of boolean operators
is available:

```certo
// Simple field comparison
require order.status == Draft

// Logical composition
require customer.status == Active
    and order.total <= customer.availableCredit

// Disjunction
require user.role == Admin
    or  user.paymentMethod.isSome()

// Negation
require not order.isRefunded

// Grouped
require (user.role == Admin or user.role == Accounts)
    and invoice.status not in [Paid, Voided]

// Optional field access — safe, never panics
require order.approvedBy.isSome()

// Temporal condition
require invoice.createdAt.age < VoidWindow

// Collection membership
require order.status in [Approved, Picking]
require order.status not in [Shipped, Delivered, Returned]
```

### The `else` Clause

The `else` clause must produce a value of the declared `errors` type.
The compiler verifies this — a type mismatch is error E0703.

```certo
// Correct — matches declared errors OrderError
rule credit_limit {
    require order.total <= customer.availableCredit
    else OrderError.CreditLimitExceeded(
        customer.availableCredit,
        order.total
    )
}

// Compile error E0703 — BillingError does not match OrderError
rule bad_error {
    require order.total <= customer.availableCredit
    else BillingError.CreditExceeded    // Error: type mismatch
}
```

Error variants can carry data — the values are evaluated in scope at
the point of violation:

```certo
else OrderError.CreditLimitExceeded(
    available: customer.availableCredit,
    required:  order.total
)
```

---

## 16.4 Rule Relationships

### `after` — Dependency Ordering

The `after` keyword declares that a rule should only be evaluated if a
named prerequisite rule passed. If the prerequisite failed or was skipped,
the dependent rule is not evaluated.

```certo
rule customer_active {
    require customer.status == Active
    else OrderError.CustomerNotActive
}

rule within_credit_limit {
    after customer_active    // only evaluated if customer_active passed
    require order.total <= customer.availableCredit
    else OrderError.CreditLimitExceeded(
        customer.availableCredit,
        order.total
    )
}
```

Multiple `after` declarations are supported — the rule is only evaluated
if all named prerequisites passed:

```certo
rule deep_check {
    after customer_active
    after has_lines
    require order.total > Money.zero(order.currency)
    else OrderError.InvalidTotal
}
```

The compiler builds a dependency graph from all `after` declarations and
detects cycles at compile time (error E0700). A cycle means two rules each
depend on the other — which makes evaluation order undefined.

### `overrides` — Rule Suspension

The `overrides` keyword declares that this rule suspends the named rule
when its own condition passes. This encodes the admin bypass pattern
cleanly — without conditional logic scattered through the validator.

```certo
rule within_credit_limit {
    require order.total <= customer.availableCredit
    else OrderError.CreditLimitExceeded(
        customer.availableCredit,
        order.total
    )
}

// When this rule's condition passes (user is Admin),
// within_credit_limit is not evaluated at all.
rule credit_limit_admin_override {
    overrides within_credit_limit
    priority 110
    require user.role == Admin
    else OrderError.NotAuthorised
}
```

The override rule itself is always evaluated. If its condition fails,
the overridden rule is evaluated normally. If its condition passes,
the overridden rule is skipped.

An override rule that always passes renders the overridden rule
unreachable — the compiler emits warning W0101.

### `priority` — Conflict Resolution

Priority is an integer declaring evaluation precedence. Higher values
are evaluated first. Default is 0.

Priority is most useful when multiple rules interact with overlapping
conditions and explicit `overrides` relationships would be verbose:

```certo
rule standard_discount_limit {
    priority 0
    require orderLine.discountPercent <= 15
    else LineError.DiscountExceedsSalesLimit
}

rule manager_discount_limit {
    priority 50
    overrides standard_discount_limit
    require user.role in [Admin, Sales]
        or  orderLine.discountPercent <= 15
    else LineError.DiscountExceedsManagerLimit
}

rule admin_discount_unlimited {
    priority 100
    overrides manager_discount_limit
    require user.role == Admin
    else LineError.NotAuthorised
}
```

The compiler emits warning W0100 when two rules conflict — one overriding
the other — without an explicit `priority` to clarify evaluation order.

---

## 16.5 Condition Expressions — Full Reference

Validator `require` clauses support the same expression grammar as the
rest of Certo, with the addition of temporal expressions.

### Field Access

```certo
require order.status == Submitted
require customer.creditLimit > Money.zero(GBP)
require order.approvedBy.isSome()       // Option field — safe access
require order.notes.isNone()            // None check
```

### Comparison Operators

```certo
require order.total == Money(d"0.00", GBP)
require order.total != Money.zero(GBP)
require order.total >  Money(d"100.00", GBP)
require order.total >= customer.creditLimit
require order.lineCount < 100
require order.lineCount <= 50
```

### Logical Operators

```certo
require customer.status == Active
    and order.total <= customer.availableCredit

require user.role == Admin
    or  user.role == Accounts

require not order.isRefunded

require (user.role == Admin or user.role == Sales)
    and customer.status == Active
```

### Collection Membership

```certo
require order.status in [Approved, Picking]
require order.status not in [Shipped, Delivered, Cancelled, Returned]
require user.role in [Admin, Sales, Accounts]
```

### Existence Checks

```certo
require order.shippingAddressId.isSome()
require order.approvedBy.isNone()
require order.notes.isSome()
```

### Cross-Field Comparison

Two fields can be compared directly. The compiler verifies they have
compatible types:

```certo
require order.total <= customer.availableCredit
require shipmentLine.quantityShipped <= orderLine.quantity
require invoice.total == order.total
require payment.amount <= invoice.amountOutstanding
```

### Temporal Expressions

The `.age` property on any `Timestamp` or `Timestamp?` field returns a
`Duration` representing elapsed time since that timestamp:

```certo
require invoice.createdAt.age < VoidWindow       // less than 30 days old
require order.submittedAt.age > ApprovalWindow   // older than 48 hours
require session.createdAt.age <= SessionExpiry   // within session window
```

`timestamp.age` is equivalent to `Timestamp.now() - timestamp`. For
`Timestamp?` fields, `.age` returns `Duration.max` when the field is
`None` — so `None.age < VoidWindow` is always false, which is the
correct default behaviour for unset timestamps.

### String Conditions

```certo
require Regex.match("^[A-Z0-9-]{3,20}$", product.sku)
require customer.email.contains("@")
require order.reference.startsWith("ORD-")
```

---

## 16.6 Calling Validators

Validators compile to a module with three functions. All three are
generated automatically — you do not write them.

### `validate` — Fail Fast

Returns on the first rule violation. Use when you want to surface one
clear error to the user.

```certo
async fn submitOrder(orderId: UUID): Result<Order, OrderError> =
    db.transaction {
        let order    = db.orders.find(orderId)?
        let customer = db.customers.find(order.customerId)?
        let user     = currentUser()?

        OrderSubmit.validate(order, context: { customer, user })?

        db.orders.update(orderId, { status: Submitted })?
        Ok(order)
    }
```

### `validateAll` — Collect All Violations

Evaluates all rules and collects all violations. Returns an empty list
if all rules pass. Use for form submission where you want to show all
errors at once.

```certo
async fn submitOrderForm(
    orderId: UUID,
    ctx:     OrderSubmitContext
): Result<Order, List<OrderError>> = {
    let order      = db.orders.find(orderId)?
    let violations = OrderSubmit.validateAll(order, context: ctx)

    match violations {
        []   => {
            db.orders.update(orderId, { status: Submitted })?
            Ok(order)
        }
        errs => Err(errs)
    }
}
```

### `validateWithDb` — Automatic Context Loading

When all context fields carry `loaded by` expressions, the compiler
generates a `validateWithDb` variant that loads context automatically
inside the current transaction. Only available when `loaded by` is
declared for every context field.

```certo
// Context declared with loaded by:
validator OrderSubmit for Order errors OrderError {
    context {
        customer: Customer  loaded by db.customers.find(order.customerId)
        user:     User      loaded by db.users.find(currentUserId())
    }
    ...
}

// validateWithDb available — no manual context building:
async fn submitOrder(orderId: UUID): Result<Order, OrderError> =
    db.transaction {
        let order = db.orders.find(orderId)?
        OrderSubmit.validateWithDb(order)?
        db.orders.update(orderId, { status: Submitted })?
        Ok(order)
    }
```

### Composing Multiple Validators

Validators are composed at the call site using sequential `?` propagation.
There is no validator merging or inheritance — composition is explicit:

```certo
async fn submitOrder(orderId: UUID): Result<Order, OrderError> =
    db.transaction {
        let order = db.orders.find(orderId)?
        let ctx   = buildContext(order)?

        // Each validator is independent — called in sequence
        OrderSubmit.validate(order, ctx)?
        BillingOrderSubmit.validate(order, ctx)?
        CreditPolicySubmit.validate(order, ctx)?

        db.orders.update(orderId, { status: Submitted })?
        Ok(order)
    }
```

The call site is the composition layer. The ordering, the error handling,
and the decision of which validators apply are all visible in one place.

---

## 16.7 Trigger Integration

A validator declared with `trigger` installs itself as a database trigger
when you run `certo db migrate`. The trigger fires `BEFORE` the DML
operation and rolls back the transaction on any rule violation.

```certo
validator OrderSubmit for Order
    errors OrderError
    trigger on Update when status == Submitted
{
    ...
}
```

### Trigger Conditions

```certo
trigger on Insert                           // fires on every INSERT
trigger on Update                           // fires on every UPDATE
trigger on Update when status == Submitted  // fires on specific value
trigger on Update when status != OLD.status // fires on any status change
```

`OLD` refers to the row before the update — equivalent to `OLD` in
PL/pgSQL trigger functions.

### User Context in Triggers

Triggers cannot receive function arguments — they fire automatically.
User context (role, id) is passed through session settings set by the
application before the DML call:

```certo
// Application sets context before the DML:
db.session.set("rule.user_id",   currentUser.id.toString())
db.session.set("rule.user_role", currentUser.role.toString())

// Then performs the DML — trigger fires automatically:
db.orders.update(orderId, { status: Submitted })
```

The compiler generates the session setting reads inside the trigger
function. The `currentUserId()` function in `loaded by` expressions
reads from the session automatically.

---

## 16.8 Named Constraints

Constraints are named boolean expressions declared at module level.
They factor out common conditions shared across multiple validators,
keeping rule bodies concise and keeping domain vocabulary consistent.

```certo
// Module-private
constraint WithinCredit =
    order.total <= customer.availableCredit

// Exported — available to importing modules
pub constraint UserIsAdmin      = user.role == Admin
pub constraint UserIsSales      = user.role in [Admin, Sales]
pub constraint UserIsAccounts   = user.role in [Admin, Accounts]
pub constraint UserIsWarehouse  = user.role in [Admin, Warehouse]
pub constraint CustomerIsActive = customer.status == Active
pub constraint ProductAvailable = product.status == Active
                               and product.stockCount > 0
```

### Deferred Resolution

Constraint bodies reference fields (`user.role`, `customer.status`) that
do not exist at the point of declaration. The compiler defers type-checking
until the constraint is used inside a validator, where the context fields
are known.

```certo
// Declared here — user.role not yet resolvable
pub constraint UserIsAdmin = user.role == Admin

// Resolved here — compiler checks:
//   1. 'user' exists in this validator's context
//   2. User type has a 'role' field
//   3. UserRole has an 'Admin' variant
validator OrderSubmit for Order errors OrderError {
    context { user: User }

    rule check_role {
        require UserIsAdmin
        else OrderError.NotAuthorised
    }
}
```

If a constraint references a field not present in the enclosing
validator's context, the compiler reports E0704 at the `require` clause —
not at the constraint declaration.

### Visibility

Constraints follow the same visibility rules as all other declarations:

```certo
// Private to this module — not importable
constraint WithinCredit = order.total <= customer.availableCredit

// Exported — importable by other modules
pub constraint UserIsAdmin = user.role == Admin
```

Importing modules use the standard import statement:

```certo
import Orders.Constraints.{ UserIsAdmin, CustomerIsActive }
import Auth.Constraints.{ UserIsAdmin, UserIsSales }
```

### Constraint Composition

Constraints can reference other constraints, building a vocabulary
of reusable conditions:

```certo
pub constraint UserIsAdmin    = user.role == Admin
pub constraint UserIsSales    = user.role in [Admin, Sales]
pub constraint UserIsAccounts = user.role in [Admin, Accounts]

// Composed from simpler constraints
pub constraint UserCanApproveOrders =
    UserIsAdmin or UserIsSales

pub constraint UserCanIssueInvoices =
    UserIsAdmin or UserIsAccounts
```

The compiler inlines all constraint references at use sites. There is
no runtime constraint object — constraints are a compile-time abstraction.

---

## 16.9 Temporal Declarations

Temporals are named `Duration` constants declared at module level.
They give business-meaningful names to time windows used in rule conditions.

```certo
pub temporal VoidWindow     = Duration.days(30)
pub temporal ReturnWindow   = Duration.days(30)
pub temporal GracePeriod    = Duration.days(3)
pub temporal ApprovalWindow = Duration.hours(48)
pub temporal SessionExpiry  = Duration.hours(8)
pub temporal QuoteValidity  = Duration.days(14)
pub temporal CreditReview   = Duration.months(12)
```

Unlike constraints, temporals are fully resolved at declaration time.
`Duration.days(30)` is a constant — no deferred resolution needed.

### Using Temporals

Temporals are used in `require` clauses via the `.age` property on
`Timestamp` fields:

```certo
rule within_void_window {
    require invoice.createdAt.age < VoidWindow
        and invoice.status not in [Paid, WrittenOff]
    else InvoiceError.VoidWindowExpired
}

rule return_eligible {
    require order.deliveredAt.age < ReturnWindow
        and order.status == Delivered
    else OrderError.ReturnWindowExpired
}
```

### The `.age` Property

`.age` is a computed property available on `Timestamp` and `Timestamp?`
fields. It returns a `Duration` representing the time elapsed since
that timestamp.

```certo
// Timestamp field — always has a value
invoice.createdAt.age        // Duration since createdAt

// Timestamp? field — may be None
order.deliveredAt.age        // Duration.max when deliveredAt is None
```

For optional timestamp fields, `.age` returns `Duration.max` when the
field is `None`. This means `noneTimestamp.age < VoidWindow` is always
`false` — the correct default when an event has not yet occurred.

The compiler verifies that `.age` is only used on `Timestamp` or
`Timestamp?` fields (error E0709).

### Visibility

Temporals follow the same visibility rules as constraints and all other
declarations:

```certo
// Private — this module only
temporal InternalWindow = Duration.hours(2)

// Exported — importable
pub temporal VoidWindow = Duration.days(30)
```

---

## 16.10 Complete Worked Example — Order Entry System

The following shows validators for a realistic order entry system.
Error types, entity types, and constraints are declared in separate
modules and imported.

### Constraints Module

```certo
// src/validators/constraints.cto
module OrderEntry.Validators.Constraints

import OrderEntry.Models.{ UserRole, CustomerStatus, ProductStatus }

// Role constraints
pub constraint UserIsAdmin     = user.role == UserRole.Admin
pub constraint UserIsSales     = user.role in [UserRole.Admin, UserRole.Sales]
pub constraint UserIsAccounts  = user.role in [UserRole.Admin, UserRole.Accounts]
pub constraint UserIsWarehouse = user.role in [UserRole.Admin, UserRole.Warehouse]

// Status constraints
pub constraint CustomerIsActive = customer.status == CustomerStatus.Active
pub constraint OrderIsDraft     = order.status == OrderStatus.Draft
pub constraint OrderIsSubmitted = order.status == OrderStatus.Submitted

// Business constraints
pub constraint WithinCreditLimit =
    order.total <= customer.availableCredit

pub constraint ProductIsAvailable =
    product.status == ProductStatus.Active
    and product.stockCount > 0
```

### Temporals Module

```certo
// src/validators/temporals.cto
module OrderEntry.Validators.Temporals

pub temporal VoidWindow     = Duration.days(30)
pub temporal ReturnWindow   = Duration.days(30)
pub temporal ApprovalWindow = Duration.hours(48)
pub temporal GracePeriod    = Duration.days(3)
pub temporal QuoteValidity  = Duration.days(14)
```

### Order Validators

```certo
// src/validators/order-validators.cto
module OrderEntry.Validators.Order

import OrderEntry.Models.{ Order, Customer, User, Address, OrderError }
import OrderEntry.Validators.Constraints.{
    UserIsAdmin, UserIsSales, CustomerIsActive, WithinCreditLimit
}
import OrderEntry.Validators.Temporals.{ ReturnWindow, ApprovalWindow }

// -----------------------------------------------------------------
// Order.submit — rules enforced when an order is submitted
// -----------------------------------------------------------------
pub validator OrderSubmit for Order
    errors OrderError
    trigger on Update when status == Submitted
{
    context {
        customer: Customer  loaded by db.customers.find(order.customerId)
        user:     User      loaded by db.users.find(currentUserId())
    }

    rule customer_active {
        require CustomerIsActive
        else OrderError.CustomerNotActive
    }

    rule has_lines {
        require order.lineCount > 0
        else OrderError.NoOrderLines
    }

    rule has_shipping_address {
        require order.shippingAddressId.isSome()
            or  order.linesAllDigital
        else OrderError.NoShippingAddress
    }

    rule has_billing_address {
        require order.billingAddressId.isSome()
        else OrderError.NoBillingAddress
    }

    rule currency_matches_customer {
        require order.currency == customer.currency
        else OrderError.CurrencyMismatch(
            order.currency,
            customer.currency
        )
    }

    rule within_credit_limit {
        after customer_active
        require WithinCreditLimit
        else OrderError.CreditLimitExceeded(
            customer.availableCredit,
            order.total
        )
    }

    rule credit_limit_admin_override {
        overrides within_credit_limit
        priority 110
        require UserIsAdmin
        else OrderError.NotAuthorised
    }
}

// -----------------------------------------------------------------
// Order.approve — rules enforced when an order is approved
// -----------------------------------------------------------------
pub validator OrderApprove for Order
    errors OrderError
    trigger on Update when status == Approved
{
    context {
        user: User loaded by db.users.find(currentUserId())
    }

    rule order_is_submitted {
        require order.status == Submitted
        else OrderError.InvalidStatusTransition(
            order.status,
            Approved
        )
    }

    rule high_value_requires_admin {
        require order.total <= Money(d"10000.00", order.currency)
            or  user.role == Admin
        else OrderError.ApprovalRequired(order.total)
    }
}

// -----------------------------------------------------------------
// Order.cancel — rules enforced when an order is cancelled
// -----------------------------------------------------------------
pub validator OrderCancel for Order
    errors OrderError
    trigger on Update when status == Cancelled
{
    rule not_yet_shipped {
        require order.status not in [Shipped, Delivered, Returned]
        else OrderError.CannotCancelShippedOrder
    }
}

// -----------------------------------------------------------------
// Order.return — rules enforced when a return is initiated
// -----------------------------------------------------------------
pub validator OrderReturn for Order
    errors OrderError
    trigger on Update when status == Returned
{
    rule is_delivered {
        require order.status == Delivered
        else OrderError.OrderNotDelivered
    }

    rule within_return_window {
        after is_delivered
        require order.deliveredAt.age < ReturnWindow
        else OrderError.ReturnWindowExpired
    }
}
```

### Invoice Validators

```certo
// src/validators/invoice-validators.cto
module OrderEntry.Validators.Invoice

import OrderEntry.Models.{ Invoice, Order, Customer, User, InvoiceError }
import OrderEntry.Validators.Constraints.{ UserIsAdmin, UserIsAccounts }
import OrderEntry.Validators.Temporals.{ VoidWindow }

// -----------------------------------------------------------------
// Invoice.issue — rules enforced when an invoice is issued
// -----------------------------------------------------------------
pub validator InvoiceIssue for Invoice
    errors InvoiceError
    trigger on Update when status == Issued
{
    context {
        order: Order  loaded by db.orders.find(invoice.orderId)
        user:  User   loaded by db.users.find(currentUserId())
    }

    rule accounts_role_required {
        require UserIsAccounts
        else InvoiceError.NotAuthorised
    }

    rule order_must_be_shipped {
        require order.status in [Shipped, Delivered]
        else InvoiceError.OrderNotShipped(order.status)
    }

    rule amount_matches_order {
        require invoice.total == order.total
        else InvoiceError.AmountMismatch(
            invoice.total,
            order.total
        )
    }
}

// -----------------------------------------------------------------
// Invoice.void — rules enforced when an invoice is voided
// -----------------------------------------------------------------
pub validator InvoiceVoid for Invoice
    errors InvoiceError
    trigger on Update when status == Voided
{
    context {
        user: User loaded by db.users.find(currentUserId())
    }

    rule within_void_window {
        require invoice.createdAt.age < VoidWindow
            and invoice.status not in [Paid, WrittenOff]
        else InvoiceError.VoidWindowExpired
    }

    rule void_admin_override {
        overrides within_void_window
        priority 110
        require UserIsAdmin
        else InvoiceError.NotAuthorised
    }
}

// -----------------------------------------------------------------
// Invoice.pay — rules enforced when payment is recorded
// -----------------------------------------------------------------
pub validator InvoicePay for Invoice
    errors InvoiceError
    trigger on Update when status == Paid
{
    context {
        user: User loaded by db.users.find(currentUserId())
    }

    rule accounts_role_required {
        require UserIsAccounts
        else InvoiceError.NotAuthorised
    }

    rule invoice_is_payable {
        require invoice.status in [Issued, PartiallyPaid, Overdue]
        else InvoiceError.NotPayable(invoice.status)
    }
}
```

### Payment Validators

```certo
// src/validators/payment-validators.cto
module OrderEntry.Validators.Payment

import OrderEntry.Models.{ Payment, Invoice, User, PaymentError }
import OrderEntry.Validators.Constraints.{ UserIsAccounts }

// -----------------------------------------------------------------
// Payment.create — rules enforced when a payment is recorded
// -----------------------------------------------------------------
pub validator PaymentCreate for Payment
    errors PaymentError
    trigger on Insert
{
    context {
        invoice: Invoice  loaded by db.invoices.find(payment.invoiceId)
        user:    User     loaded by db.users.find(currentUserId())
    }

    rule accounts_role_required {
        require UserIsAccounts
        else PaymentError.NotAuthorised
    }

    rule invoice_is_payable {
        require invoice.status in [Issued, PartiallyPaid, Overdue]
        else PaymentError.InvoiceNotPayable(invoice.status)
    }

    rule amount_within_balance {
        after invoice_is_payable
        require payment.amount <= invoice.amountOutstanding
        else PaymentError.ExceedsOutstandingBalance(
            payment.amount,
            invoice.amountOutstanding
        )
    }

    rule currency_matches_invoice {
        require payment.currency == invoice.customer.currency
        else PaymentError.CurrencyMismatch(
            payment.currency,
            invoice.customer.currency
        )
    }
}
```

### Order Line Validators

```certo
// src/validators/order-line-validators.cto
module OrderEntry.Validators.OrderLine

import OrderEntry.Models.{ OrderLine, Order, Product, User, LineError }
import OrderEntry.Validators.Constraints.{
    UserIsAdmin, UserIsSales, ProductIsAvailable, OrderIsDraft
}

// -----------------------------------------------------------------
// OrderLine.create — rules enforced when a line is added
// -----------------------------------------------------------------
pub validator OrderLineCreate for OrderLine
    errors LineError
    trigger on Insert
{
    context {
        order:   Order   loaded by db.orders.find(orderLine.orderId)
        product: Product loaded by db.products.find(orderLine.productId)
        user:    User    loaded by db.users.find(currentUserId())
    }

    rule order_is_editable {
        require OrderIsDraft
        else LineError.OrderNotEditable(order.status)
    }

    rule product_is_available {
        require ProductIsAvailable
        else LineError.ProductNotAvailable(
            product.id,
            product.status
        )
    }

    rule quantity_within_stock {
        after product_is_available
        require orderLine.quantity <= product.stockCount
        else LineError.InsufficientStock(
            product.stockCount,
            orderLine.quantity
        )
    }
}

// -----------------------------------------------------------------
// OrderLine.apply_discount — rules enforced when discount is applied
// -----------------------------------------------------------------
pub validator OrderLineApplyDiscount for OrderLine
    errors LineError
{
    context {
        product: Product loaded by db.products.find(orderLine.productId)
        user:    User    loaded by db.users.find(currentUserId())
    }

    rule discount_requires_reason {
        require orderLine.discountReason.isSome()
        else LineError.DiscountReasonRequired
    }

    rule discount_within_sales_limit {
        require UserIsAdmin
            or  orderLine.discountPercent <= d"15.00"
        else LineError.DiscountExceedsSalesLimit(
            orderLine.discountPercent,
            d"15.00"
        )
    }

    rule discount_not_below_zero {
        require orderLine.discountPercent >= d"0.00"
        else LineError.NegativeDiscount
    }

    rule discount_not_exceed_line {
        require orderLine.discountAmount <= orderLine.unitPrice
        else LineError.DiscountExceedsLineValue(
            orderLine.discountAmount,
            orderLine.unitPrice
        )
    }
}
```

---

## 16.11 Migration Guide — From Guards to Validators

If you currently use `guard` clauses in your functions to enforce business
rules, this section shows how to migrate to validators.

### Before — Guards in Functions

```certo
async fn submitOrder(orderId: UUID): Result<Order, OrderError> =
    db.transaction {
        let order    = db.orders.find(orderId)?
        let customer = db.customers.find(order.customerId)?
        let user     = currentUser()?

        guard customer.status == Active
            else OrderError.CustomerNotActive

        guard order.lineCount > 0
            else OrderError.NoOrderLines

        guard order.total <= customer.availableCredit
                or user.role == Admin
            else OrderError.CreditLimitExceeded(
                customer.availableCredit,
                order.total
            )

        db.orders.update(orderId, { status: Submitted })?
        Ok(order)
    }
```

### After — Validator

```certo
pub validator OrderSubmit for Order errors OrderError {
    context {
        customer: Customer  loaded by db.customers.find(order.customerId)
        user:     User      loaded by db.users.find(currentUserId())
    }

    rule customer_active {
        require customer.status == Active
        else OrderError.CustomerNotActive
    }

    rule has_lines {
        require order.lineCount > 0
        else OrderError.NoOrderLines
    }

    rule within_credit_limit {
        require order.total <= customer.availableCredit
        else OrderError.CreditLimitExceeded(
            customer.availableCredit,
            order.total
        )
    }

    rule credit_limit_admin_override {
        overrides within_credit_limit
        priority 110
        require user.role == Admin
        else OrderError.NotAuthorised
    }
}

async fn submitOrder(orderId: UUID): Result<Order, OrderError> =
    db.transaction {
        let order = db.orders.find(orderId)?
        OrderSubmit.validateWithDb(order)?
        db.orders.update(orderId, { status: Submitted })?
        Ok(order)
    }
```

### What You Gain

**Named rules** — each rule has an identity. Error messages, test output,
and documentation all reference the rule by name.

**Independent testability** — each rule can be tested in isolation
(see section 16.12).

**Compiler-verified relationships** — `after` and `overrides` replace
imperative ordering logic. The compiler verifies the dependency graph.

**DB trigger installation** — add `trigger on Update when status == Submitted`
to install the same rules at the database layer automatically.

**Generated documentation** — `certo doc` produces a rule catalogue
from your validator declarations.

---

## 16.12 Testing Validators

Validators integrate with Certo's test runner. Each rule can be tested
in isolation using the `ruleTest` block.

### Rule Tests

```certo
ruleTest OrderSubmit.customer_active "passes for active customer" {
    entity: Order { ..defaultOrder }
    context: OrderSubmitContext {
        customer: Customer { ..defaultCustomer, status: Active }
        user:     defaultUser
    }
    expect: pass
}

ruleTest OrderSubmit.customer_active "fails for suspended customer" {
    entity: Order { ..defaultOrder }
    context: OrderSubmitContext {
        customer: Customer { ..defaultCustomer, status: Suspended }
        user:     defaultUser
    }
    expect: fail with OrderError.CustomerNotActive
}
```

### Validator Tests

Test all rules at once with a full context:

```certo
validatorTest OrderSubmit "passes for valid order" {
    entity:  validDraftOrder
    context: OrderSubmitContext { customer: activeCustomer, user: salesUser }
    expect:  pass
}

validatorTest OrderSubmit "fails when credit limit exceeded" {
    entity:  Order { ..validDraftOrder, total: Money(d"99999.00", GBP) }
    context: OrderSubmitContext { customer: Customer { ..activeCustomer, availableCredit: Money(d"100.00", GBP) }
               user: salesUser }
    expect:  fail with OrderError.CreditLimitExceeded
}

validatorTest OrderSubmit "admin bypasses credit limit" {
    entity:  Order { ..validDraftOrder, total: Money(d"99999.00", GBP) }
    context: OrderSubmitContext { customer: Customer { ..activeCustomer, availableCredit: Money(d"100.00", GBP) }
               user: adminUser }
    expect:  pass
}
```

### Running Validator Tests

```
certo test --filter OrderSubmit
certo test validators/
certo test --coverage
```

---

## 16.13 Compiler Error Reference

| Code  | Category  | Description |
|-------|-----------|-------------|
| E0700 | Validator | Cycle detected in `after` dependency graph |
| E0701 | Validator | `after` references unknown rule in this validator |
| E0702 | Validator | `overrides` references unknown rule in this validator |
| E0703 | Validator | `else` branch type does not match declared `errors` type |
| E0704 | Validator | Constraint field not in scope — not declared in `context` |
| E0705 | Validator | `loaded by` expression type does not match context field type |
| E0706 | Validator | `trigger` field does not exist on entity |
| E0707 | Validator | `trigger` value is not a valid variant of the field type |
| E0708 | Validator | `temporal` declaration does not resolve to `Duration` |
| E0709 | Validator | `.age` used on non-`Timestamp` field |
| W0100 | Validator | Rule conflict without explicit `priority` or `overrides` |
| W0101 | Validator | Unreachable rule — permanently shadowed by `overrides` |
| W0102 | Validator | `validateWithDb` unavailable — not all context fields have `loaded by` |

### Error Message Examples

**E0700 — Cycle in dependency graph**

```
error[E0700]: cycle detected in rule dependency graph
  → src/validators/order-validators.cto:34:9
   │
34 │     after within_credit_limit
   │     ^^^^^^^^^^^^^^^^^^^^^^^^^
   │
   = note: cycle: customer_active → within_credit_limit → customer_active
   = help: remove one of the `after` declarations to break the cycle
```

**E0703 — Error type mismatch**

```
error[E0703]: `else` branch produces wrong error type
  → src/validators/order-validators.cto:28:14
   │
28 │         else BillingError.CreditExceeded
   │              ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   │
   = expected: OrderError  (declared on validator at line 12)
   = found:    BillingError
   = help: change the error variant to an OrderError variant,
           or change the validator's `errors` declaration
```

**E0704 — Constraint field not in scope**

```
error[E0704]: constraint references field `user` not in validator context
  → src/validators/order-validators.cto:41:17
   │
41 │         require UserIsAdmin
   │                 ^^^^^^^^^^^
   │
   = note: UserIsAdmin references `user.role`
   = note: `user` is not declared in this validator's context block
   = help: add `user: User` to the context block:
           context {
               user: User  loaded by db.users.find(currentUserId())
           }
```

**E0709 — `.age` on non-Timestamp field**

```
error[E0709]: `.age` can only be used on Timestamp or Timestamp? fields
  → src/validators/invoice-validators.cto:19:17
   │
19 │         require invoice.status.age < VoidWindow
   │                 ^^^^^^^^^^^^^^^^^^
   │
   = found type: InvoiceStatus  (not a Timestamp)
   = help: `.age` computes elapsed time — use it on a Timestamp field
           such as `invoice.createdAt.age`
```

---

## 16.14 Summary — New Declarations

Section 16 introduces four new top-level declaration kinds and five
new keywords within validator bodies.

### New Top-Level Declarations

| Declaration | Syntax | Resolved |
|---|---|---|
| `constraint` | `pub? constraint Name = bool_expr` | At use site in validator |
| `temporal` | `pub? temporal Name = Duration` | At declaration time |
| `validator` | `pub? validator Name for Type errors ErrorType { ... }` | At compile time |
| `rule` | `rule name { require ... else ... }` (inside validator) | At compile time |

### New Keywords

| Keyword | Context | Meaning |
|---|---|---|
| `after` | Inside `rule` | This rule only runs if named rule passed |
| `overrides` | Inside `rule` | Suspends named rule when this condition passes |
| `priority` | Inside `rule` | Evaluation precedence — higher wins |
| `loaded by` | Inside `context` field | DB expression to load this context field |
| `trigger` | On `validator` | Install as DB trigger via `certo db migrate` |

### New Test Blocks

| Block | Purpose |
|---|---|
| `ruleTest` | Tests a single named rule in isolation |
| `validatorTest` | Tests all rules in a validator with a full context |
