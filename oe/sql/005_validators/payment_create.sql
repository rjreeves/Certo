-- =============================================================================
-- Validator: Payment.create
-- Generated: see git history for generation date
-- DO NOT EDIT — regenerate from YAML definitions
-- =============================================================================

-- Rules enforced (4):
--   payment_requires_accounts_role: Only accounts users can record payments
--   payment_requires_issued_invoice: Payments can only be recorded against issued or overdue invoices
--   payment_cannot_exceed_outstanding_balance: A payment cannot exceed the outstanding invoice balance
--   payment_currency_must_match_invoice: Payment currency must match the invoice currency

CREATE OR REPLACE FUNCTION validate_payments_create(
    p_record  JSONB,
    p_context JSONB
) RETURNS VOID AS $$
DECLARE
    v_user_role TEXT;
BEGIN
    v_user_role := p_context->>'user_role';

    -- Only accounts users can record payments
    IF NOT ((p_context->>'user_role') = 'ADMIN'
    OR (p_context->>'user_role') = 'ACCOUNTS') THEN
        PERFORM raise_rule_violation('NOT_AUTHORISED_PAYMENT', 'Only accounts users can record payments');
    END IF;

    -- Payments can only be recorded against issued or overdue invoices
    IF NOT ((p_record->>'status') IN ('ISSUED', 'PARTIALLY_PAID', 'OVERDUE')) THEN
        PERFORM raise_rule_violation('INVOICE_NOT_PAYABLE', 'Payments can only be recorded against issued or overdue invoices');
    END IF;

    -- A payment cannot exceed the outstanding invoice balance
    IF NOT ((p_record->>'amount') <= (p_record->>'amount_outstanding')::NUMERIC) THEN
        PERFORM raise_rule_violation('PAYMENT_EXCEEDS_BALANCE', 'Payment amount cannot exceed the outstanding invoice balance');
    END IF;

    -- Payment currency must match the invoice currency
    IF NOT ((p_record->>'currency') = (p_record->>'currency')::NUMERIC) THEN
        PERFORM raise_rule_violation('PAYMENT_CURRENCY_MISMATCH', 'Payment currency must match the customer''s trading currency');
    END IF;

END;
$$ LANGUAGE plpgsql;

-- Convenience wrapper reads context from session settings.
CREATE OR REPLACE FUNCTION validate_payments_create(p_record JSONB)
RETURNS VOID AS $$
BEGIN
PERFORM validate_payments_create(p_record, current_rule_context());
END;
$$ LANGUAGE plpgsql;
