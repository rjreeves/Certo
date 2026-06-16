-- =============================================================================
-- API Helper Functions
-- Generated: 2026-06-15 19:50:44 UTC
-- DO NOT EDIT — regenerate from YAML definitions
-- =============================================================================

-- Pre-flight check wrappers — return JSONB instead of raising.
-- Use from REST handlers before committing DML.

-- validate_safe: generic wrapper
CREATE OR REPLACE FUNCTION validate_safe(
    p_func    TEXT,
    p_record  JSONB,
    p_context JSONB
) RETURNS JSONB AS $$
DECLARE
    v_parts TEXT[];
BEGIN
    EXECUTE format('SELECT %I($1, $2)', p_func)
        USING p_record, p_context;
    RETURN jsonb_build_object('valid', TRUE, 'violations', '[]'::JSONB);
EXCEPTION WHEN OTHERS THEN
    v_parts := string_to_array(SQLERRM, '|');
    RETURN jsonb_build_object(
        'valid', FALSE,
        'violations', jsonb_build_array(jsonb_build_object(
            'code',    v_parts[1],
            'message', COALESCE(v_parts[2], SQLERRM)
        ))
    );
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: CreditNote.create
CREATE OR REPLACE FUNCTION preflight_credit_notes_create(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_credit_notes_create', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: Discount.create
CREATE OR REPLACE FUNCTION preflight_discounts_create(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_discounts_create', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: Invoice.create
CREATE OR REPLACE FUNCTION preflight_invoices_create(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_invoices_create', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: Invoice.issue
CREATE OR REPLACE FUNCTION preflight_invoices_issue(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_invoices_issue', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: Invoice.void
CREATE OR REPLACE FUNCTION preflight_invoices_void(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_invoices_void', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: Order.approve
CREATE OR REPLACE FUNCTION preflight_orders_approve(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_orders_approve', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: Order.cancel
CREATE OR REPLACE FUNCTION preflight_orders_cancel(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_orders_cancel', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: Order.create
CREATE OR REPLACE FUNCTION preflight_orders_create(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_orders_create', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: Order.return
CREATE OR REPLACE FUNCTION preflight_orders_return(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_orders_return', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: Order.set_priority
CREATE OR REPLACE FUNCTION preflight_orders_set_priority(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_orders_set_priority', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: Order.submit
CREATE OR REPLACE FUNCTION preflight_orders_submit(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_orders_submit', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: OrderLine.apply_discount
CREATE OR REPLACE FUNCTION preflight_order_lines_apply_discount(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_order_lines_apply_discount', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: OrderLine.create
CREATE OR REPLACE FUNCTION preflight_order_lines_create(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_order_lines_create', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: OrderLine.update
CREATE OR REPLACE FUNCTION preflight_order_lines_update(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_order_lines_update', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: Payment.create
CREATE OR REPLACE FUNCTION preflight_payments_create(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_payments_create', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: Shipment.create
CREATE OR REPLACE FUNCTION preflight_shipments_create(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_shipments_create', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: Shipment.dispatch
CREATE OR REPLACE FUNCTION preflight_shipments_dispatch(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_shipments_dispatch', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

-- Pre-flight: ShipmentLine.create
CREATE OR REPLACE FUNCTION preflight_shipment_lines_create(
    p_record  JSONB,
    p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
    RETURN validate_safe('validate_shipment_lines_create', p_record, p_context);
END;
$$ LANGUAGE plpgsql;
