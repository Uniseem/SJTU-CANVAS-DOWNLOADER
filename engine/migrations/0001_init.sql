-- Preferences that are not credentials (JSON values by key).
CREATE TABLE app_settings (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- One row per file to download. A lesson with two tracks is two rows.
CREATE TABLE downloads (
    id TEXT PRIMARY KEY NOT NULL,
    -- Canvas profile id of the account that queued the task.
    owner_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('video', 'file')),
    course_id TEXT NOT NULL,
    course_name TEXT NOT NULL,
    -- Lesson id (video) or Canvas file id (file).
    resource_id TEXT NOT NULL,
    -- slides | teacher | composite for videos.
    track TEXT,
    title TEXT NOT NULL,
    begin_time TEXT,
    -- The folder the user chose; files go into course sub-folders below it.
    destination TEXT NOT NULL,
    -- Final path once the first transfer allocated a free name; the partial
    -- file is this path plus ".part" until the transfer completes.
    file_path TEXT,
    status TEXT NOT NULL CHECK (status IN ('queued', 'downloading', 'paused', 'completed', 'failed', 'cancelled')),
    received INTEGER NOT NULL DEFAULT 0,
    total INTEGER,
    etag TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT
);

CREATE INDEX downloads_by_status ON downloads (status, created_at);
