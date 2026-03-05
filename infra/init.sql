-- pekko-agent PostgreSQL schema
-- Episodic Memory & Audit Log

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- Agent Events (Event Sourcing)
CREATE TABLE agent_events (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    event_type  VARCHAR(100) NOT NULL,
    agent_id    VARCHAR(100) NOT NULL,
    session_id  UUID NOT NULL,
    tenant_id   VARCHAR(100) NOT NULL,
    correlation_id UUID NOT NULL,
    payload     JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_events_agent ON agent_events(agent_id, created_at DESC);
CREATE INDEX idx_events_session ON agent_events(session_id, created_at);
CREATE INDEX idx_events_tenant ON agent_events(tenant_id, created_at DESC);
CREATE INDEX idx_events_type ON agent_events(event_type);

-- Episodic Memory
CREATE TABLE episodes (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    agent_id    VARCHAR(100) NOT NULL,
    session_id  UUID NOT NULL,
    action_taken TEXT NOT NULL,
    reasoning   TEXT NOT NULL,
    outcome     TEXT NOT NULL,
    context     TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_episodes_agent ON episodes(agent_id, created_at DESC);

-- Audit Log
CREATE TABLE audit_log (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id   VARCHAR(100) NOT NULL,
    agent_id    VARCHAR(100) NOT NULL,
    action      VARCHAR(200) NOT NULL,
    resource    VARCHAR(200) NOT NULL,
    outcome     VARCHAR(50) NOT NULL,
    details     JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_tenant ON audit_log(tenant_id, created_at DESC);
CREATE INDEX idx_audit_agent ON audit_log(agent_id, created_at DESC);

-- EHS Domain Tables
CREATE TABLE permits (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    permit_id   VARCHAR(50) UNIQUE NOT NULL,
    title       VARCHAR(500) NOT NULL,
    facility_id VARCHAR(100) NOT NULL,
    tenant_id   VARCHAR(100) NOT NULL,
    status      VARCHAR(50) NOT NULL DEFAULT 'pending',
    industry    VARCHAR(100),
    issued_date DATE,
    expiry_date DATE,
    metadata    JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE inspections (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    inspection_id   VARCHAR(50) UNIQUE NOT NULL,
    facility_id     VARCHAR(100) NOT NULL,
    tenant_id       VARCHAR(100) NOT NULL,
    inspection_type VARCHAR(100) NOT NULL,
    status          VARCHAR(50) NOT NULL DEFAULT 'scheduled',
    risk_score      REAL,
    scheduled_date  DATE,
    completed_date  DATE,
    inspector       VARCHAR(200),
    findings        JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE compliance_records (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    regulation_id   VARCHAR(100) NOT NULL,
    facility_id     VARCHAR(100) NOT NULL,
    tenant_id       VARCHAR(100) NOT NULL,
    status          VARCHAR(50) NOT NULL DEFAULT 'pending',
    score           REAL,
    findings        JSONB,
    remediation     JSONB,
    checked_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
