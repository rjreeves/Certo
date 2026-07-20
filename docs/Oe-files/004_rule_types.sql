-- =============================================================================
-- Rule Validation Infrastructure
-- Generated: 2026-06-15 19:50:44 UTC
-- DO NOT EDIT — regenerate from YAML definitions
-- =============================================================================

-- ---------------------------------------------------------------------------
-- raise_rule_violation(code, message)
-- Structured exception — format: RULE_CODE|Human message
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION raise_rule_violation(p_code TEXT, p_message TEXT)
RETURNS VOID AS $$
BEGIN
    RAISE EXCEPTION USING
        ERRCODE = 'P0001',
        MESSAGE = p_code || '|' || p_message,
        HINT    = p_message;
END;
$$ LANGUAGE plpgsql;

-- ---------------------------------------------------------------------------
-- current_rule_context()
-- Reads context from session settings set by application before DML.
-- Application sets: SET LOCAL rule.user_id = '...';
--                   SET LOCAL rule.user_role = 'ADMIN';
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION current_rule_context()
RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object(
        'user_id',   current_setting('rule.user_id',   TRUE),
        'user_role', current_setting('rule.user_role', TRUE)
    );
END;
$$ LANGUAGE plpgsql STABLE;

-- ---------------------------------------------------------------------------
-- record_age_days(ts TIMESTAMPTZ) → NUMERIC
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION record_age_days(ts TIMESTAMPTZ)
RETURNS NUMERIC AS $$
BEGIN
    RETURN EXTRACT(EPOCH FROM (NOW() - ts)) / 86400.0;
END;
$$ LANGUAGE plpgsql STABLE;
