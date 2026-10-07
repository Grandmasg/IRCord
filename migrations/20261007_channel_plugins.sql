-- Per kanaal uitgeschakelde plugins (!plugin disable <naam>). Alleen uitgeschakelde combinaties staan hier.
CREATE TABLE IF NOT EXISTS channel_plugins (
    channel TEXT NOT NULL,
    plugin TEXT NOT NULL,
    PRIMARY KEY (channel, plugin)
);
