-- =============================================================================
-- Validator: Invoice.void
-- Generated: 2026-06-15 19:50:44 UTC
-- DO NOT EDIT — regenerate from YAML definitions
-- =============================================================================

-- Rules enforced (2):
--   invoice_void_within_window: An invoice can only be voided within the standard void window
--   invoice_void_admin_override: Admin users can void invoices outside the standard void window

CREATE OR REPLACE FUNCTION validate_invoices_void(
    p_record  JSONB,
    p_context JSONB
) RETURNS VOID AS $$
DECLARE
    v_user_role TEXT;
    v_invoice_void_admin_override_active BOOLEAN := FALSE;
BEGIN
    v_user_role := p_context->>'user_role';

    -- Admin users can void invoices outside the standard void window
    -- (override — suspends 'invoice_void_within_window' when true)
    IF (p_context->>'user_role') = 'ADMIN' THEN
        v_invoice_void_admin_override_active := TRUE;
    END IF;

    -- An invoice can only be voided within the standard void window
    IF NOT v_invoice_void_admin_override_active THEN
        IF NOT ((record_age_days((p_record->>'invoice.created_at')::TIMESTAMPTZ) < 30)
        AND ((p_record->>'status') NOT IN ('PAID', 'WRITTEN_OFF'))) THEN
            PERFORM raise_rule_violation('INVOICE_VOID_WINDOW_EXPIRED', 'Invoices cannot be voided after 30 days or once paid');
        END IF;
    END IF;

END;
$$ LANGUAGE plpgsql;

-- Convenience wrapper reads context from session settings.
CREATE OR REPLACE FUNCTION validate_invoices_void(p_record JSONB)
RETURNS VOID AS $$
BEGIN
    PERFORM validate_invoices_void(p_record, current_rule_context());
END;
$$ LANGUAGE plpgsql;
