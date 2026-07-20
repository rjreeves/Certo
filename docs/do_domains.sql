/*
 *   Copyright (c) 2026 
 *   All rights reserved.
 */
-- =====================================================
-- Core Validation Domains
-- =====================================================

CREATE DOMAIN email_address AS text
CHECK (
    VALUE ~* '^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$'
);

CREATE DOMAIN phone_number AS text
CHECK (
    VALUE ~ '^[+0-9() -]{7,25}$'
);

CREATE DOMAIN postal_code AS text
CHECK (
    length(trim(VALUE)) BETWEEN 3 AND 12
);

CREATE DOMAIN url AS text
CHECK (
    VALUE ~* '^https?://.+'
);

CREATE DOMAIN color_code AS text
CHECK (
    VALUE ~* '^#[0-9A-F]{6}$'
);

CREATE DOMAIN username AS text
CHECK (
    length(VALUE) BETWEEN 3 AND 32
    AND VALUE ~ '^[a-zA-Z0-9_-]+$'
);

CREATE DOMAIN password_hash AS text
CHECK (
    length(VALUE) >= 32
);

-- =====================================================
-- Numeric Domains
-- =====================================================

CREATE DOMAIN money_amount AS numeric(19,4)
CHECK (
    VALUE >= 0
);

CREATE DOMAIN percentage AS numeric(5,2)
CHECK (
    VALUE BETWEEN 0 AND 100
);

CREATE DOMAIN positive_integer AS integer
CHECK (
    VALUE > 0
);

CREATE DOMAIN non_negative_integer AS integer
CHECK (
    VALUE >= 0
);

CREATE DOMAIN quantity AS numeric(19,4)
CHECK (
    VALUE >= 0
);

CREATE DOMAIN tax_rate AS numeric(5,4)
CHECK (
    VALUE BETWEEN 0 AND 1
);

-- =====================================================
-- Identifier Domains
-- =====================================================

CREATE DOMAIN uuid_key AS uuid
NOT NULL;

-- =====================================================
-- Geographic Domains
-- =====================================================

CREATE DOMAIN state_code AS char(2)
CHECK (
    VALUE ~ '^[A-Z]{2}$'
);

CREATE DOMAIN country_code AS char(2)
CHECK (
    VALUE ~ '^[A-Z]{2}$'
);

-- =====================================================
-- Financial Domains
-- =====================================================

CREATE DOMAIN currency_code AS char(3)
CHECK (
    VALUE ~ '^[A-Z]{3}$'
);

-- =====================================================
-- Date Domains
-- =====================================================

CREATE DOMAIN date_of_birth AS date
CHECK (
    VALUE BETWEEN DATE '1900-01-01'
          AND CURRENT_DATE
);

CREATE DOMAIN future_date AS date
CHECK (
    VALUE >= CURRENT_DATE
);

-- =====================================================
-- Status Domains
-- =====================================================

CREATE DOMAIN status_code AS text
CHECK (
    VALUE IN (
        'ACTIVE',
        'INACTIVE',
        'PENDING',
        'SUSPENDED',
        'DELETED'
    )
);

-- =====================================================
-- Email Address
-- =====================================================

CREATE DOMAIN valid_email AS text
CHECK (
    VALUE ~* '^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$'
);

-- =====================================================
-- Date of Birth
-- =====================================================

CREATE DOMAIN date_of_birth AS date
CHECK (
    VALUE BETWEEN DATE '1900-01-01'
          AND CURRENT_DATE
);

-- =====================================================
-- Positive Integer
-- =====================================================

CREATE DOMAIN positive_integer AS integer
CHECK (
    VALUE > 0
);

-- =====================================================
-- URL
-- =====================================================

CREATE DOMAIN url AS text
CHECK (
    VALUE ~* '^https?://.+'
);

-- =====================================================
-- Hex Color Code
-- =====================================================

CREATE DOMAIN color_code AS text
CHECK (
    VALUE ~* '^#[0-9A-F]{6}$'
);

-- =====================================================
-- Status Value
-- =====================================================

CREATE DOMAIN status_value AS text
CHECK (
    VALUE IN (
        'ACTIVE',
        'INACTIVE',
        'PENDING',
        'SUSPENDED',
        'DELETED'
    )
);



-- Identity and contact
CREATE DOMAIN d_email AS TEXT
    CHECK (VALUE ~ '^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$');

CREATE DOMAIN d_phone AS TEXT
    CHECK (VALUE ~ '^\+?[1-9]\d{7,14}$');

CREATE DOMAIN d_uuid AS UUID
    NOT NULL;

-- Name fields
CREATE DOMAIN d_short_name AS VARCHAR(100)
    NOT NULL
    CHECK (LENGTH(TRIM(VALUE)) > 0);

CREATE DOMAIN d_long_name AS VARCHAR(255)
    NOT NULL
    CHECK (LENGTH(TRIM(VALUE)) > 0);

-- Financial
CREATE DOMAIN d_positive_money AS NUMERIC(19,4)
    NOT NULL
    CHECK (VALUE > 0);

CREATE DOMAIN d_non_negative_money AS NUMERIC(19,4)
    NOT NULL
    CHECK (VALUE >= 0);

CREATE DOMAIN d_percentage AS NUMERIC(5,2)
    NOT NULL
    CHECK (VALUE BETWEEN 0 AND 100);

CREATE DOMAIN d_currency_code AS CHAR(3)
    NOT NULL
    CHECK (VALUE ~ '^[A-Z]{3}$');

-- Address
CREATE DOMAIN d_zip_code AS CHAR(5)
    CHECK (VALUE ~ '^\d{5}$');

CREATE DOMAIN d_country_code AS CHAR(2)
    NOT NULL
    CHECK (VALUE ~ '^[A-Z]{2}$');

CREATE DOMAIN d_address_line AS VARCHAR(200)
    CHECK (LENGTH(TRIM(VALUE)) > 0);

-- Dates and status
CREATE DOMAIN d_future_timestamp AS TIMESTAMPTZ
    CHECK (VALUE > NOW());

CREATE DOMAIN d_positive_int AS INTEGER
    NOT NULL
    CHECK (VALUE > 0);

CREATE DOMAIN d_payment_terms_days AS INTEGER
    NOT NULL
    CHECK (VALUE IN (7, 14, 30, 45, 60, 90));

-- Reference codes
CREATE DOMAIN d_invoice_number AS TEXT
    NOT NULL
    CHECK (VALUE ~ '^INV-[0-9]{6}$');

CREATE DOMAIN d_tax_id AS TEXT
    CHECK (VALUE ~ '^\d{2}-\d{7}$');