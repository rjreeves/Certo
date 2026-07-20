-- =============================================================================
-- Rule Enforcement Triggers
-- Generated: 2026-06-15 19:50:44 UTC
-- DO NOT EDIT — regenerate from YAML definitions
-- =============================================================================

-- BEFORE INSERT/UPDATE triggers calling validator functions.

-- CreditNote
CREATE OR REPLACE FUNCTION trg_validate_credit_notes()
RETURNS TRIGGER AS $$
DECLARE
    v_record  JSONB;
    v_context JSONB;
BEGIN
    v_record  := row_to_json(NEW)::JSONB;
    v_context := current_rule_context();
    IF TG_OP = 'INSERT' THEN
        PERFORM validate_credit_notes_create(v_record, v_context);
    ELSIF TG_OP = 'UPDATE' THEN
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_validate_credit_notes ON credit_notes;
CREATE TRIGGER trg_validate_credit_notes
    BEFORE INSERT OR UPDATE ON credit_notes
    FOR EACH ROW
    EXECUTE FUNCTION trg_validate_credit_notes();

-------------------------------------------------------------------------------

-- Discount
CREATE OR REPLACE FUNCTION trg_validate_discounts()
RETURNS TRIGGER AS $$
DECLARE
    v_record  JSONB;
    v_context JSONB;
BEGIN
    v_record  := row_to_json(NEW)::JSONB;
    v_context := current_rule_context();
    IF TG_OP = 'INSERT' THEN
        PERFORM validate_discounts_create(v_record, v_context);
    ELSIF TG_OP = 'UPDATE' THEN
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_validate_discounts ON discounts;
CREATE TRIGGER trg_validate_discounts
    BEFORE INSERT OR UPDATE ON discounts
    FOR EACH ROW
    EXECUTE FUNCTION trg_validate_discounts();

-------------------------------------------------------------------------------

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
        PERFORM validate_invoices_create(v_record, v_context);
    ELSIF TG_OP = 'UPDATE' THEN
        IF OLD.status IS DISTINCT FROM NEW.status THEN
            CASE NEW.status
                WHEN 'ISSUED' THEN
                    PERFORM validate_invoices_issue(v_record, v_context);
                WHEN 'VOIDED' THEN
                    PERFORM validate_invoices_void(v_record, v_context);
                ELSE NULL;
            END CASE;
        ELSE
            NULL;
        END IF;
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
        PERFORM validate_orders_create(v_record, v_context);
    ELSIF TG_OP = 'UPDATE' THEN
        IF OLD.status IS DISTINCT FROM NEW.status THEN
            CASE NEW.status
                WHEN 'SUBMITTED' THEN
                    PERFORM validate_orders_submit(v_record, v_context);
                WHEN 'APPROVED' THEN
                    PERFORM validate_orders_approve(v_record, v_context);
                WHEN 'CANCELLED' THEN
                    PERFORM validate_orders_cancel(v_record, v_context);
                WHEN 'RETURNED' THEN
                    PERFORM validate_orders_return(v_record, v_context);
                ELSE NULL;
            END CASE;
        ELSE
            PERFORM validate_orders_set_priority(v_record, v_context);
        END IF;
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
        PERFORM validate_order_lines_create(v_record, v_context);
    ELSIF TG_OP = 'UPDATE' THEN
        PERFORM validate_order_lines_update(v_record, v_context);
        PERFORM validate_order_lines_apply_discount(v_record, v_context);
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

-- Shipment
CREATE OR REPLACE FUNCTION trg_validate_shipments()
RETURNS TRIGGER AS $$
DECLARE
    v_record  JSONB;
    v_context JSONB;
BEGIN
    v_record  := row_to_json(NEW)::JSONB;
    v_context := current_rule_context();
    IF TG_OP = 'INSERT' THEN
        PERFORM validate_shipments_create(v_record, v_context);
    ELSIF TG_OP = 'UPDATE' THEN
        IF OLD.status IS DISTINCT FROM NEW.status THEN
            CASE NEW.status
                WHEN 'DISPATCHED' THEN
                    PERFORM validate_shipments_dispatch(v_record, v_context);
                ELSE NULL;
            END CASE;
        ELSE
            NULL;
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_validate_shipments ON shipments;
CREATE TRIGGER trg_validate_shipments
    BEFORE INSERT OR UPDATE ON shipments
    FOR EACH ROW
    EXECUTE FUNCTION trg_validate_shipments();

-------------------------------------------------------------------------------

-- ShipmentLine
CREATE OR REPLACE FUNCTION trg_validate_shipment_lines()
RETURNS TRIGGER AS $$
DECLARE
    v_record  JSONB;
    v_context JSONB;
BEGIN
    v_record  := row_to_json(NEW)::JSONB;
    v_context := current_rule_context();
    IF TG_OP = 'INSERT' THEN
        PERFORM validate_shipment_lines_create(v_record, v_context);
    ELSIF TG_OP = 'UPDATE' THEN
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_validate_shipment_lines ON shipment_lines;
CREATE TRIGGER trg_validate_shipment_lines
    BEFORE INSERT OR UPDATE ON shipment_lines
    FOR EACH ROW
    EXECUTE FUNCTION trg_validate_shipment_lines();

-------------------------------------------------------------------------------
