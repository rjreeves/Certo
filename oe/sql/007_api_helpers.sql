-- =============================================================================
-- API Helper Functions
-- Generated: see git history for generation date
-- DO NOT EDIT — regenerate from YAML definitions
-- =============================================================================

-- Pre-flight check wrappers — return JSONB instead of raising.
-- Use from REST handlers before committing DML.

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

-- Pre-flight: Invoice.void
CREATE OR REPLACE FUNCTION preflight_invoices_void(
p_record  JSONB,
p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
RETURN validate_safe('validate_invoices_void', p_record, p_context);
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

-- Pre-flight: Payment.create
CREATE OR REPLACE FUNCTION preflight_payments_create(
p_record  JSONB,
p_context JSONB DEFAULT current_rule_context()
) RETURNS JSONB AS $$
BEGIN
RETURN validate_safe('validate_payments_create', p_record, p_context);
END;
$$ LANGUAGE plpgsql;

