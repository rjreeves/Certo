-- =============================================================================
-- Validator: Order.submit
-- Generated: see git history for generation date
-- DO NOT EDIT — regenerate from YAML definitions
-- =============================================================================

-- Rules enforced (6):
--   customer_must_be_active_to_order: Orders can only be placed for customers with ACTIVE status
--   order_must_have_lines_to_submit: An order must have at least one line before it can be submitted
--   order_must_have_shipping_address: A physical order must have a shipping address before submission
--   order_must_have_billing_address: An order must have a billing address before submission
--   credit_limit_override_requires_admin: Submitting an order that exceeds the credit limit requires admin approval
--   customer_credit_limit_not_exceeded: A submitted order must not cause the customer's outstanding balance to exceed their credit limit

CREATE OR REPLACE FUNCTION validate_orders_submit(
    p_record  JSONB,
    p_context JSONB
) RETURNS VOID AS $$
DECLARE
    v_user_role TEXT;
    v_customer_must_be_active_to_order_passed BOOLEAN := FALSE;
    v_credit_limit_override_requires_admin_active BOOLEAN := FALSE;
BEGIN
    v_user_role := p_context->>'user_role';

    -- Submitting an order that exceeds the credit limit requires admin approval (override — suspends 'customer_credit_limit_not_exceeded' when true)
    IF (p_context->>'user_role') = 'ADMIN' THEN
        v_credit_limit_override_requires_admin_active := TRUE;
    END IF;

    -- Orders can only be placed for customers with ACTIVE status
    IF NOT ((p_record->>'status') = 'ACTIVE') THEN
        PERFORM raise_rule_violation('CUSTOMER_NOT_ACTIVE', 'Orders cannot be placed for customers who are not active');
    END IF;
    v_customer_must_be_active_to_order_passed := ((p_record->>'status') = 'ACTIVE');

    -- An order must have at least one line before it can be submitted
    IF NOT ((p_record->>'line_count')::NUMERIC > 0) THEN
        PERFORM raise_rule_violation('ORDER_HAS_NO_LINES', 'An order must have at least one product line before submission');
    END IF;

    -- A physical order must have a shipping address before submission
    IF NOT ((p_record->>'shipping_address_id') IS NOT NULL
    OR (p_record->>'lines_all_digital')::BOOLEAN = TRUE) THEN
        PERFORM raise_rule_violation('NO_SHIPPING_ADDRESS', 'A shipping address is required before submitting this order');
    END IF;

    -- An order must have a billing address before submission
    IF NOT ((p_record->>'billing_address_id') IS NOT NULL) THEN
        PERFORM raise_rule_violation('NO_BILLING_ADDRESS', 'A billing address is required before submitting this order');
    END IF;

    -- A submitted order must not cause the customer's outstanding balance to exceed their credit limit
    IF v_customer_must_be_active_to_order_passed THEN
        IF NOT v_credit_limit_override_requires_admin_active THEN
            IF NOT ((p_record->>'total') <= (p_record->>'available_credit')::NUMERIC) THEN
                PERFORM raise_rule_violation('CREDIT_LIMIT_EXCEEDED', 'This order would exceed the customer''s available credit limit');
            END IF;
        END IF;
    END IF;

END;
$$ LANGUAGE plpgsql;

-- Convenience wrapper reads context from session settings.
CREATE OR REPLACE FUNCTION validate_orders_submit(p_record JSONB)
RETURNS VOID AS $$
BEGIN
PERFORM validate_orders_submit(p_record, current_rule_context());
END;
$$ LANGUAGE plpgsql;
