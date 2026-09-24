-- Rebuild the external-content FTS index after making entry upserts
-- update rows in place. This repairs rows orphaned by older INSERT OR REPLACE
-- behavior and indexes entries created before the FTS table existed.
INSERT INTO session_search_fts(session_search_fts) VALUES('rebuild');
