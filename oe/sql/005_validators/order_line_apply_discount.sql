-- =============================================================================
-- Validator: OrderLine.apply_discount
-- Generated: see git history for generation date
-- DO NOT EDIT — regenerate from YAML definitions
-- =============================================================================

-- Rules enforced (2):
--   discount_cannot_make_line_negative: A discount cannot make an order line total negative
--   discount_percent_within_sales_limit: Sales users cannot apply a line discount exceeding 15%

CREATE OR REPLACE FUNCTION validate_order_lines_apply_discount(
    p_record  JSONB,
    p_context JSONB
) RETURNS VOID AS $$
DECLARE
    v_user_role TEXT;
BEGIN
    v_user_role := p_context->>'user_role';

    -- A discount cannot make an order line total negative
    IF NOT ((p_record->>'discount_amount') <= (p_record->>'unit_price')::NUMERIC) THEN
        PERFORM raise_rule_violation('DISCOUNT_EXCEEDS_LINE_VALUE', 'Discount cannot exceed the line value');
    END IF;

    -- Sales users cannot apply a line discount exceeding 15%
    IF NOT ((p_context->>'user_role') = 'ADMIN'
    OR (p_record->>'discount_percent')::NUMERIC <= 15) THEN
        PERFORM raise_rule_violation('DISCOUNT_EXCEEDS_SALES_LIMIT', 'Sales users cannot apply discounts above 15%. Admin approval required.');
    END IF;

END;
$$ LANGUAGE plpgsql;

-- Convenience wrapper reads context from session settings.
CREATE OR REPLACE FUNCTION validate_order_lines_apply_discount(p_record JSONB)
RETURNS VOID AS $$
BEGIN
PERFORM validate_order_lines_apply_discount(p_record, current_rule_context());
END;
$$ LANGUAGE plpgsql;
