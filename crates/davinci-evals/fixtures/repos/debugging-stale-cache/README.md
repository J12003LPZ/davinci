# Debugging hard fixture: stale cache

The cache key does not include the revision, so an old response can survive a
data change. Clearing every entry or disabling the cache is symptom suppression.
