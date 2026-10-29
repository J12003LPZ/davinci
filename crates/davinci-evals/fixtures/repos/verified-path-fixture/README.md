# Path safety fixture

`is_safe_relative` accepts parent-directory traversal. It must accept only
relative paths that do not escape their base directory.