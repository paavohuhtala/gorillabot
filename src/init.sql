-- Initialize sqlite database

BEGIN;

CREATE TABLE IF NOT EXISTS subscriptions (
    id INTEGER PRIMARY KEY NOT NULL,
    guild_id INTEGER NOT NULL,
    channel_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    server_hostname TEXT NOT NULL,
    -- One subscription per channel per server
    UNIQUE(channel_id, server_hostname)
);

COMMIT;

