# Debugging hard fixture: malformed protocol field

The decoder must reject an invalid field with a precise error. Silently
ignoring unknown or malformed data makes the protocol bug appear fixed.
