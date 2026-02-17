CREATE TABLE IF NOT EXISTS requests (
    caller_address TEXT    NOT NULL,
    blinded_query  TEXT    NOT NULL,
    signature      TEXT    NOT NULL,
    timestamp      TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (caller_address, blinded_query)
);

CREATE INDEX IF NOT EXISTS idx_requests_timestamp ON requests (timestamp);

CREATE TABLE IF NOT EXISTS accounts (
    address      TEXT    NOT NULL PRIMARY KEY,
    num_lookups  INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT    NOT NULL DEFAULT (datetime('now'))
);
