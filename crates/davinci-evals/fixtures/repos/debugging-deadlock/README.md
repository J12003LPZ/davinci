# Debugging hard fixture: deadlock

The lock acquisition order is inverted between the two operations. Increasing
timeouts or adding sleeps does not repair the causal lock-order bug.
