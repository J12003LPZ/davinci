# Debugging hard fixture: deterministic race

Two updates can observe the same sequence number. The repair must establish a
real ordering or atomic invariant; sleeps and serial-test flags are forbidden.
