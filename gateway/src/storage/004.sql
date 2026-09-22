CREATE TABLE identity_users (
id TEXT PRIMARY KEY, login TEXT NOT NULL UNIQUE,
password_hash TEXT NOT NULL, password_salt TEXT NOT NULL,
enabled INTEGER NOT NULL DEFAULT 1, administrator INTEGER NOT NULL DEFAULT 0,
created_at INTEGER NOT NULL);
CREATE TABLE identity_keys (
id TEXT PRIMARY KEY, user_id TEXT REFERENCES identity_users(id),
name TEXT NOT NULL, token_hash TEXT NOT NULL UNIQUE,
revoked INTEGER NOT NULL DEFAULT 0, expires_at INTEGER, created_at INTEGER NOT NULL);
CREATE INDEX identity_keys_user ON identity_keys(user_id);
CREATE TABLE identity_sessions (
token_hash TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES identity_users(id),
expires_at INTEGER NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX identity_sessions_user ON identity_sessions(user_id);
