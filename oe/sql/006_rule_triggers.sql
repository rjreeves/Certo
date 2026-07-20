-- =============================================================================
-- Rule Triggers
-- Generated: see git history for generation date
-- DO NOT EDIT — regenerate from YAML definitions
-- =============================================================================

-- Invoice
CREATE OR REPLACE FUNCTION trg_validate_invoices()
RETURNS TRIGGER AS $$
DECLARE
v_record  JSONB;
v_context JSONB;
BEGIN
v_record  := row_to_json(NEW)::JSONB;
v_context := current_rule_context();
    IF TG_OP = 'INSERT' THEN
        NULL;
    ELSIF TG_OP = 'UPDATE' THEN
        NULL; -- named-action validators called via preflight_* API helpers
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_validate_invoices ON invoices;
CREATE TRIGGER trg_validate_invoices
BEFORE INSERT OR UPDATE ON invoices
FOR EACH ROW
EXECUTE FUNCTION trg_validate_invoices();

-------------------------------------------------------------------------------

-- Order
CREATE OR REPLACE FUNCTION trg_validate_orders()
RETURNS TRIGGER AS $$
DECLARE
v_record  JSONB;
v_context JSONB;
BEGIN
v_record  := row_to_json(NEW)::JSONB;
v_context := current_rule_context();
    IF TG_OP = 'INSERT' THEN
        NULL;
    ELSIF TG_OP = 'UPDATE' THEN
        NULL; -- named-action validators called via preflight_* API helpers
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_validate_orders ON orders;
CREATE TRIGGER trg_validate_orders
BEFORE INSERT OR UPDATE ON orders
FOR EACH ROW
EXECUTE FUNCTION trg_validate_orders();

-------------------------------------------------------------------------------

-- OrderLine
CREATE OR REPLACE FUNCTION trg_validate_order_lines()
RETURNS TRIGGER AS $$
DECLARE
v_record  JSONB;
v_context JSONB;
BEGIN
v_record  := row_to_json(NEW)::JSONB;
v_context := current_rule_context();
    IF TG_OP = 'INSERT' THEN
        NULL;
    ELSIF TG_OP = 'UPDATE' THEN
        NULL; -- named-action validators called via preflight_* API helpers
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_validate_order_lines ON order_lines;
CREATE TRIGGER trg_validate_order_lines
BEFORE INSERT OR UPDATE ON order_lines
FOR EACH ROW
EXECUTE FUNCTION trg_validate_order_lines();

-------------------------------------------------------------------------------

-- Payment
CREATE OR REPLACE FUNCTION trg_validate_payments()
RETURNS TRIGGER AS $$
DECLARE
v_record  JSONB;
v_context JSONB;
BEGIN
v_record  := row_to_json(NEW)::JSONB;
v_context := current_rule_context();
    IF TG_OP = 'INSERT' THEN
        PERFORM validate_payments_create(v_record, v_context);
    ELSIF TG_OP = 'UPDATE' THEN
        NULL; -- named-action validators called via preflight_* API helpers
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_validate_payments ON payments;
CREATE TRIGGER trg_validate_payments
BEFORE INSERT OR UPDATE ON payments
FOR EACH ROW
EXECUTE FUNCTION trg_validate_payments();

-------------------------------------------------------------------------------

