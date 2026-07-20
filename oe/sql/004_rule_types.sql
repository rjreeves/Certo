-- =============================================================================
-- Rule Validation Infrastructure
-- Generated: see git history for generation date
-- DO NOT EDIT — regenerate from YAML definitions
-- =============================================================================

-- raise_rule_violation(code, message)
-- Raises a structured exception: 'RULE_CODE|Human readable message'
CREATE OR REPLACE FUNCTION raise_rule_violation(
    p_code    TEXT,
    p_message TEXT
) RETURNS VOID AS $$
BEGIN
    RAISE EXCEPTION USING
        ERRCODE = 'P0001',
        MESSAGE = p_code || '|' || p_message,
        HINT    = p_message;
END;
$$ LANGUAGE plpgsql;

-- current_rule_context()
-- Returns JSONB context set by the application via SET LOCAL.
--   SET LOCAL rule.user_id   = '<uuid>';
--   SET LOCAL rule.user_role = 'ADMIN';
CREATE OR REPLACE FUNCTION current_rule_context()
RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object(
        'user_id',   current_setting('rule.user_id',   TRUE),
        'user_role', current_setting('rule.user_role', TRUE)
    );
END;
$$ LANGUAGE plpgsql STABLE;

-- record_age_days(ts) → NUMERIC
-- How many days ago a timestamp occurred.
CREATE OR REPLACE FUNCTION record_age_days(ts TIMESTAMPTZ)
RETURNS NUMERIC AS $$
BEGIN
    RETURN EXTRACT(EPOCH FROM (NOW() - ts)) / 86400.0;
END;
$$ LANGUAGE plpgsql STABLE;
