# Benchmarks

This is a stub. It is filled in at M1 with criterion results for the matching core.

Report, with the full method (machine, input distribution, warmup):

Throughput, in orders per second through the matching core.

Tail latency per submit, at p50, p99, and p99.9.

Allocation behavior. The criterion harness asserts that the steady state match path does no per iteration heap allocation and only bounded amortized allocation overall.

State clearly that these are matching core microbenchmarks, not end to end numbers. Any figure quoted in the README carries the same caveat and links back here.
