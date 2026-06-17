create table if not exists artifact_blobs (
    execution_id text not null references session_executions(execution_id) on delete cascade,
    ref text not null,
    mime text not null,
    size_bytes bigint not null,
    sha256 text not null,
    data bytea not null,
    created_at timestamptz not null default now(),
    primary key (execution_id, ref)
);

create unique index if not exists session_events_artifact_capture_dedupe_idx
    on session_events (execution_id, ((payload->>'artifact_id')))
    where event_type = 'artifact.captured'
      and execution_id is not null
      and payload ? 'artifact_id';
