/*
 *   Copyright (c) 2026 
 *   All rights reserved.
 */
-- =============================================================================
-- Order Entry Schema
-- =============================================================================

-- ---------------------------------------------------------------------------
-- Extensions
-- ---------------------------------------------------------------------------

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- ---------------------------------------------------------------------------
-- Enum types
-- ---------------------------------------------------------------------------

CREATE TYPE user_role AS ENUM (
    'ADMIN',
    'SALES',
    'ACCOUNTS',
    'WAREHOUSE'
);

CREATE TYPE order_status AS ENUM (
    'DRAFT',
    'SUBMITTED',
    'FULFILLED',
    'CANCELLED'
);

CREATE TYPE invoice_status AS ENUM (
    'DRAFT',
    'ISSUED',
    'PARTIALLY_PAID',
    'PAID',
    'OVERDUE',
    'VOIDED',
    'WRITTEN_OFF'
);

CREATE TYPE payment_status AS ENUM (
    'PENDING',
    'CLEARED',
    'REVERSED'
);

CREATE TYPE customer_status AS ENUM (
    'ACTIVE',
    'INACTIVE',
    'SUSPENDED'
);

-- ---------------------------------------------------------------------------
-- Customers
-- ---------------------------------------------------------------------------

CREATE TABLE customers (
    id                  UUID            PRIMARY KEY DEFAULT gen_random_uuid(),
    name                TEXT            NOT NULL,
    status              customer_status NOT NULL DEFAULT 'ACTIVE',
    credit_limit        NUMERIC(18, 4)  NOT NULL DEFAULT 0,
    outstanding_balance NUMERIC(18, 4)  NOT NULL DEFAULT 0,
    -- available_credit is a computed column used by validators
    currency            CHAR(3)         NOT NULL DEFAULT 'GBP',
    created_at          TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

-- Validators read available_credit from the record JSONB.
-- This view exposes it so row_to_json(NEW) includes it in triggers.
CREATE OR REPLACE VIEW customers_with_credit AS
    SELECT *,
           (credit_limit - outstanding_balance) AS available_credit
    FROM customers;

-- ---------------------------------------------------------------------------
-- Addresses
-- ---------------------------------------------------------------------------

CREATE TABLE addresses (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id UUID        NOT NULL REFERENCES customers(id),
    line1       TEXT        NOT NULL,
    line2       TEXT,
    city        TEXT        NOT NULL,
    postcode    TEXT        NOT NULL,
    country     CHAR(2)     NOT NULL DEFAULT 'GB',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- Products
-- ---------------------------------------------------------------------------

CREATE TABLE products (
    id          UUID            PRIMARY KEY DEFAULT gen_random_uuid(),
    sku         TEXT            NOT NULL UNIQUE,
    name        TEXT            NOT NULL,
    unit_price  NUMERIC(18, 4)  NOT NULL,
    is_digital  BOOLEAN         NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- Orders
--
-- Validator columns (populated via triggers / computed on write):
--   status, line_count, lines_all_digital,
--   shipping_address_id, billing_address_id,
--   total, available_credit (from customer snapshot)
-- ---------------------------------------------------------------------------

CREATE TABLE orders (
    id                  UUID            PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id         UUID            NOT NULL REFERENCES customers(id),
    status              order_status    NOT NULL DEFAULT 'DRAFT',

    -- Address links — nullable until set before submission
    shipping_address_id UUID            REFERENCES addresses(id),
    billing_address_id  UUID            REFERENCES addresses(id),

    -- Denormalised summary columns kept in sync by triggers
    line_count          INT             NOT NULL DEFAULT 0,
    lines_all_digital   BOOLEAN         NOT NULL DEFAULT FALSE,
    subtotal            NUMERIC(18, 4)  NOT NULL DEFAULT 0,
    discount_total      NUMERIC(18, 4)  NOT NULL DEFAULT 0,
    total               NUMERIC(18, 4)  NOT NULL DEFAULT 0,

    -- Snapshot of customer credit at point of submission (for audit)
    available_credit    NUMERIC(18, 4),

    notes               TEXT,
    created_at          TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- Order Lines
--
-- Validator columns:
--   unit_price, discount_percent, discount_amount, total
-- ---------------------------------------------------------------------------

CREATE TABLE order_lines (
    id               UUID            PRIMARY KEY DEFAULT gen_random_uuid(),
    order_id         UUID            NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    product_id       UUID            NOT NULL REFERENCES products(id),

    unit_price       NUMERIC(18, 4)  NOT NULL,
    quantity         INT             NOT NULL DEFAULT 1 CHECK (quantity > 0),
    discount_percent NUMERIC(5, 2)   NOT NULL DEFAULT 0 CHECK (discount_percent >= 0),
    discount_amount  NUMERIC(18, 4)  NOT NULL DEFAULT 0,
    subtotal         NUMERIC(18, 4)  NOT NULL,   -- unit_price * quantity
    total            NUMERIC(18, 4)  NOT NULL,   -- subtotal - discount_amount
    is_digital       BOOLEAN         NOT NULL DEFAULT FALSE,

    created_at       TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- Invoices
--
-- Validator columns:
--   status, amount, amount_outstanding, currency, created_at
-- ---------------------------------------------------------------------------

CREATE TABLE invoices (
    id                UUID            PRIMARY KEY DEFAULT gen_random_uuid(),
    order_id          UUID            NOT NULL REFERENCES orders(id),
    customer_id       UUID            NOT NULL REFERENCES customers(id),
    status            invoice_status  NOT NULL DEFAULT 'DRAFT',

    amount            NUMERIC(18, 4)  NOT NULL,
    amount_paid       NUMERIC(18, 4)  NOT NULL DEFAULT 0,
    amount_outstanding NUMERIC(18, 4) NOT NULL,   -- kept in sync by payment triggers

    currency          CHAR(3)         NOT NULL DEFAULT 'GBP',

    issued_at         TIMESTAMPTZ,
    due_at            TIMESTAMPTZ,
    voided_at         TIMESTAMPTZ,
    voided_by         UUID,           -- user id

    created_at        TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- Payments
--
-- Validator columns:
--   amount, amount_outstanding (from invoice), currency, status
-- ---------------------------------------------------------------------------

CREATE TABLE payments (
    id                 UUID            PRIMARY KEY DEFAULT gen_random_uuid(),
    invoice_id         UUID            NOT NULL REFERENCES invoices(id),
    customer_id        UUID            NOT NULL REFERENCES customers(id),
    status             payment_status  NOT NULL DEFAULT 'PENDING',

    amount             NUMERIC(18, 4)  NOT NULL CHECK (amount > 0),
    currency           CHAR(3)         NOT NULL DEFAULT 'GBP',

    -- Snapshot of invoice state at time of payment (for validator JSONB)
    amount_outstanding NUMERIC(18, 4)  NOT NULL,

    reference          TEXT,
    notes              TEXT,

    created_at         TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- Users
-- ---------------------------------------------------------------------------

CREATE TABLE users (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    email       TEXT        NOT NULL UNIQUE,
    name        TEXT        NOT NULL,
    role        user_role   NOT NULL DEFAULT 'SALES',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- Indexes
-- ---------------------------------------------------------------------------

CREATE INDEX idx_orders_customer_id      ON orders(customer_id);
CREATE INDEX idx_orders_status           ON orders(status);
CREATE INDEX idx_order_lines_order_id    ON order_lines(order_id);
CREATE INDEX idx_invoices_order_id       ON invoices(order_id);
CREATE INDEX idx_invoices_customer_id    ON invoices(customer_id);
CREATE INDEX idx_invoices_status         ON invoices(status);
CREATE INDEX idx_payments_invoice_id     ON payments(invoice_id);
CREATE INDEX idx_addresses_customer_id   ON addresses(customer_id);

-- ---------------------------------------------------------------------------
-- Order summary maintenance trigger
-- Keeps orders.line_count, lines_all_digital, subtotal, discount_total, total
-- in sync whenever order_lines are inserted, updated, or deleted.
-- ---------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION trg_sync_order_summary()
RETURNS TRIGGER AS $$
DECLARE
    v_order_id UUID;
BEGIN
    v_order_id := COALESCE(NEW.order_id, OLD.order_id);

    UPDATE orders SET
        line_count        = (SELECT COUNT(*)    FROM order_lines WHERE order_id = v_order_id),
        lines_all_digital = (SELECT BOOL_AND(is_digital) FROM order_lines WHERE order_id = v_order_id),
        subtotal          = (SELECT COALESCE(SUM(subtotal), 0) FROM order_lines WHERE order_id = v_order_id),
        discount_total    = (SELECT COALESCE(SUM(discount_amount), 0) FROM order_lines WHERE order_id = v_order_id),
        total             = (SELECT COALESCE(SUM(total), 0) FROM order_lines WHERE order_id = v_order_id),
        updated_at        = NOW()
    WHERE id = v_order_id;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_sync_order_summary
    AFTER INSERT OR UPDATE OR DELETE ON order_lines
    FOR EACH ROW
    EXECUTE FUNCTION trg_sync_order_summary();

-- ---------------------------------------------------------------------------
-- Invoice outstanding balance maintenance trigger
-- Keeps invoices.amount_outstanding in sync after each payment.
-- ---------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION trg_sync_invoice_balance()
RETURNS TRIGGER AS $$
DECLARE
    v_invoice_id UUID;
    v_paid       NUMERIC(18, 4);
    v_amount     NUMERIC(18, 4);
BEGIN
    v_invoice_id := COALESCE(NEW.invoice_id, OLD.invoice_id);

    SELECT amount,
           COALESCE(SUM(p.amount) FILTER (WHERE p.status = 'CLEARED'), 0)
      INTO v_amount, v_paid
      FROM invoices i
      LEFT JOIN payments p ON p.invoice_id = i.id
     WHERE i.id = v_invoice_id
     GROUP BY i.amount;

    UPDATE invoices SET
        amount_paid        = v_paid,
        amount_outstanding = GREATEST(v_amount - v_paid, 0),
        status = CASE
            WHEN v_paid >= v_amount              THEN 'PAID'::invoice_status
            WHEN v_paid > 0                      THEN 'PARTIALLY_PAID'::invoice_status
            ELSE status
        END,
        updated_at = NOW()
    WHERE id = v_invoice_id;

    -- Also update customer outstanding balance.
    UPDATE customers SET
        outstanding_balance = (
            SELECT COALESCE(SUM(amount_outstanding), 0)
            FROM invoices
            WHERE customer_id = (SELECT customer_id FROM invoices WHERE id = v_invoice_id)
              AND status NOT IN ('PAID', 'VOIDED', 'WRITTEN_OFF')
        ),
        updated_at = NOW()
    WHERE id = (SELECT customer_id FROM invoices WHERE id = v_invoice_id);

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_sync_invoice_balance
    AFTER INSERT OR UPDATE ON payments
    FOR EACH ROW
    EXECUTE FUNCTION trg_sync_invoice_balance();

-- ---------------------------------------------------------------------------
-- updated_at auto-maintenance
-- ---------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION trg_set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER set_updated_at BEFORE UPDATE ON customers    FOR EACH ROW EXECUTE FUNCTION trg_set_updated_at();
CREATE TRIGGER set_updated_at BEFORE UPDATE ON orders       FOR EACH ROW EXECUTE FUNCTION trg_set_updated_at();
CREATE TRIGGER set_updated_at BEFORE UPDATE ON order_lines  FOR EACH ROW EXECUTE FUNCTION trg_set_updated_at();
CREATE TRIGGER set_updated_at BEFORE UPDATE ON invoices     FOR EACH ROW EXECUTE FUNCTION trg_set_updated_at();
CREATE TRIGGER set_updated_at BEFORE UPDATE ON payments     FOR EACH ROW EXECUTE FUNCTION trg_set_updated_at();
CREATE TRIGGER set_updated_at BEFORE UPDATE ON products     FOR EACH ROW EXECUTE FUNCTION trg_set_updated_at();
CREATE TRIGGER set_updated_at BEFORE UPDATE ON users        FOR EACH ROW EXECUTE FUNCTION trg_set_updated_at();

SET search_path TO Oe, public;
