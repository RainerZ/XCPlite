# Calibration RCU: per-page reference counting

Companion to [CAL_RCU.md](CAL_RCU.md), which describes the three-page RCU pattern that the original
`ves-mecaapi` calibration implementation was derived from.

This document states why that pattern needed to be improved for `ves-mecaapi`, shows the replacement as
pseudo code in the same style, and lists the risks it carries.

It implements **suggestion 1 of CAL_RCU.md** ("Implement full RCU with a reclamation list of free pages, to
avoid the non deterministic visibility delays and starvation of calibration updates") while keeping the
property that CAL_RCU.md rightly emphasises: **the memory requirement stays independent of the number of
application threads.** It also makes **suggestion 2** (an `owned` segment mode to escape the second-lock
behaviour) unnecessary, because the second-lock behaviour is gone for every segment without a mode flag.

## Why the three-page pattern had to be improved

CAL_RCU.md openly documents the compromises (its points 2, 3 and 4): visibility only in the *second* lock,
non-deterministic visibility delay, and possible starvation. Those are acceptable trade-offs for a
tool-driven parameter write. In `ves-mecaapi` they had four consequences that go beyond a delay.

**1. A callback-only consumer received exactly one update, ever.**
`ves-mecaapi` added an update callback so an application can be notified instead of polling. The free page is
only ever handed back by a reader (`free_page = ecu_page` inside `lock`). An application that consumes only
the callback never locks, so after the first publish `free_page` stays `NULL` forever, every later
`try_publish` fails, and no further callback is ever invoked. The pending write is not lost — it stays in the
writer page — but it is never announced. This is a defect, not a latency compromise.

**2. Reads were not monotonic.**
Because publish and hand-over are decoupled, the read following a successful publish could return an *older*
payload than the one just published and reported to the callback. A consumer had no contract it could rely on.

**3. The coherence that the `lock_count == 0` gate is supposed to buy was not actually delivered.**
The three stores in `lock` are not atomic with respect to the `lock_count` increment that gates them:

    T3: lock_count++ returns 0, enters the hand-over branch, is preempted before "ecu_page = ecu_page_next"
    T1: lock_count++ returns 1, skips hand-over, reads ecu_page      -> old page
    T3: resumes, stores ecu_page = ecu_page_next
    T1: nested lock, skips hand-over, reads ecu_page                 -> new page

T1 holds two pages of different generations inside one read-side critical section. So the pattern paid the
second-lock visibility delay for a guarantee it did not provide, at either thread or nesting scope.

**4. One leaked lock count was permanently fatal.**
A reader that never unlocks (thread terminated inside its critical section) leaves `lock_count >= 1` forever.
No hand-over can then ever occur again, so *no reader on any thread* ever sees a new payload again and
publishing stops for that block, permanently and silently.

## The replacement, as pseudo code

Reclamation is driven by a reference count per page instead of by reader progress. Reader identity is a
private per-thread nesting depth, which the writer never inspects, so no registry of readers is needed.

```
    Shared mutable atomic state between the writer thread and the ECU application threads is:
        - latest:       packed {generation, page index} of the currently published page
        - page_refs[N]: one reference count per page

    Writer owned, not shared:
        - xcp_page:     page permanently owned by the writer, accumulates pending changes
        - scan_cursor:  round robin start of the publish target search

    Per reader thread, private, non atomic, never read by the writer:
        - depth:        read side nesting depth
        - pinned:       page pinned while depth > 0


// Multithreaded lock, lock-free
function lock(segment) {
    if (depth == 0) {                             // only the outermost lock pins
        loop {
            stamp = latest (seq_cst)
            page  = page_of(stamp)
            page_refs[page]++ (seq_cst)           // announce the pin
            if (latest (seq_cst) == stamp) {      // still published => writer cannot target it
                pinned = page
                break
            }
            page_refs[page]-- (seq_cst)           // a publish raced us, retry
        }
    }
    depth++
    return pinned                                 // nested locks return the same page
}

// Multithreaded unlock, wait-free
function unlock(segment) {
    depth--
    if (depth == 0) {
        page_refs[pinned]-- (seq_cst)
    }
}

// Single threaded write
function write(segment, offset, data[]) {
    xcp_page[offset] = data
    if (!consistency_hold) {
        try_publish(segment)
    }
}

// Single threaded publish
function try_publish(segment) -> bool {
    stamp     = latest (seq_cst)
    published = page_of(stamp)

    // The reclamation list: scan round robin for a page that is neither the writer page,
    // nor the published page, nor pinned by any reader
    target = NONE
    page   = scan_cursor
    repeat N times {
        if (page != xcp_page && page != published) {
            if (page_refs[page] (seq_cst) == 0) {  // the writer side evaluation of the pin
                target = page
                break
            }
        }
        page = (page + 1) mod N
    }

    if (target == NONE) {
        return false                              // every candidate is pinned
    }

    memcpy(target, xcp_page)                      // no page swap, xcp_page keeps accumulating
    latest (seq_cst) = pack(generation_of(stamp) + 1, target)
    scan_cursor = (target + 1) mod N              // do not reuse a page before the other candidates
    return true
}
```

## Why it is correct

- The writer never selects the published page as a publish target.
- A reader only uses a page after re-validating that it is *still* the published page, so the writer cannot be
  copying into it.
- The generation half of the stamp rules out the ABA case where a page is retired and later re-published under
  the same index while a slow reader is still validating.
- The reference counts need to be **atomic** for two independent reasons. First, several reader threads
  increment and decrement the same counter concurrently, so a non-atomic read-modify-write would lose counts
  and a pinned page could appear free. Second, the counter is the state the writer evaluates when it looks
  for a publish target.
- All four operations that couple reader and writer - the reader's increment and its validating load, the
  writer's reference count load and its publication store - are **sequentially consistent**. The argument
  needs a single total order over them, otherwise the writer may read `refs == 0` while the reader's
  validating load still reads the old stamp, and both proceed: the writer copies into a page the reader has
  just accepted. This is the store-buffering pattern, which acquire/release does *not* close. It is stricter
  than the acquire/release note in CAL_RCU.md and is the one place where a weaker ordering silently breaks
  the algorithm.

## What this buys

- A successful publish is visible to every read-side critical section entered afterwards, on any thread,
  regardless of locks held concurrently by other threads. No second-lock delay, no cross-reader coupling.
- Reads are monotonic: a read never observes an older payload than an earlier read did.
- Nested reads are coherent, for real this time, because `pinned` is captured once and re-read by nobody.
- Publishing is independent of reader progress, so a callback-only consumer is notified about every update.
- Memory stays bounded by the page count, **independent of the thread count** - unlike a classical
  hazard-pointer RCU, which needs O(reader threads) pages.

## Risks and compromises

1. **The outermost lock is lock-free, not wait-free.** Its validation loop retries when a publish lands
   concurrently. Retries are bounded in practice by the publish rate against the read-section length, not by a
   hard limit. Nested locks and unlock are wait-free.
2. **Publish can still be refused**, but only while all `N-2` candidates are pinned, which needs `N-2`
   concurrent readers that each entered in a *different* publication generation. It then degrades to exactly
   the old behaviour - the change is kept in the writer page and retried on the cyclic timer (250 ms in
   `ves-mecaapi`) - so there is no correctness cliff.
3. **A reader that never unlocks permanently retires one page.** `PageCount - 2` such leaks stop publishing, so
   `PageCount - 3` are tolerated - none at all with three pages. Better than the old pattern, where a single leak
   was immediately fatal for reads *and* publishing, but still silent.
4. **Memory cost: two extra pages per block** (see below).
5. **`thread_local` for the per-reader state** needs a MISRA C++:2023 6.7.1 deviation (block-scope thread
   storage duration). The state is POD, constant-initialized and trivially destructible, so no dynamic
   initialization, guard variable or thread-exit destructor is involved. If the library is built as a shared
   object, the TLS model must be pinned to `initial-exec`, otherwise the first access per thread goes through
   `__tls_get_addr` and may allocate.
6. **Generation wraparound** after 2^32 publishes per block.
7. **No cross-thread generation coherence.** Two application threads may compute on different generations
   within the same cycle. This was already true of the three-page pattern (see point 3 above), so it is not a
   regression - but it is not solved either, and an application that needs it must synchronise itself.

## Memory

Measured `sizeof` of the RCU state per calibration block, old three-page layout against the new five-page
layout. The overhead is exactly two extra pages minus 8 bytes, because the new bookkeeping (one 64 bit
publication stamp, one reference count per page, one round-robin cursor) is 8 bytes smaller than the old one:

| Payload | 3 pages (old) | 5 pages (new) | Overhead |
|--------:|--------------:|--------------:|---------:|
|    16 B |          96 B |         120 B |    +24 B |
|   256 B |         816 B |        1320 B |   +504 B |
|   1 KiB |        3120 B |        5160 B |  +2040 B |
|   4 KiB |       12336 B |       20520 B |  +8184 B |
|  16 KiB |       49200 B |       81960 B | +32760 B |

The immutable reference page is a `static` member per calibration block type and is unaffected.

The page count is the last template parameter of `CalibrationBlock`, defaulted, so it can be chosen per block:

```cpp
// Default page count.
ves::mc::CalibrationBlock<kName, MyParameters> block{};

// Three pages, the structural minimum, for a block where memory matters more than publish throughput.
ves::mc::CalibrationBlock<kName, MyParameters, &ves::mc::internal::CreateReferencePage<MyParameters>, 3U> block{};
```

Fewer than three pages is rejected at compile time by a `static_assert` in both `CalibrationBlock` and `Rcu`.

Measured with `test/cal_test` (see below), 4 readers, one writer:

| Pages | 1 KiB block | Deferred publishes, paced writer | Deferred, burst writer | Readers that may leak a pin |
|------:|------------:|---------------------------------:|-----------------------:|----------------------------:|
|     3 |      3104 B |                           0.00 % |                   64 % |                           0 |
|     4 |      4128 B |                           0.00 % |                    7 % |                           1 |
|     5 |      5160 B |                           0.00 % |                  0.3 % |                           2 |

Three pages is sufficient for correctness and for the defect this change fixes. What the extra pages buy is publish
throughput when the tool writes in bursts, and tolerance against a reader that never releases: with three pages a
single such reader blocks publishing permanently.

## Verification

### Unit tests

48/48 pass under Debug, TSan, UBSan and ASan/LSan. The `Rcu__*` and `CalibrationBlock__UpdateCallback*` cases cover:
a callback-only consumer receiving every update, a read inside the callback already observing the published value,
repeated publishes with no reader at all, publish visibility while another reader holds a page, nesting coherence
across a publish, publish refusal only when every candidate is pinned, and a block configured with the minimum
page count.

### Test application

`test/cal_test` is the equivalent of the test application CAL_RCU.md describes, so the numbers below are directly
comparable to the ones published there. Parameters are compile time settings with the same names, set through CMake
cache variables:

```
cmake -DCAL_TEST_PAGE_COUNT=3 -DCAL_TEST_THREAD_COUNT=8 -DCAL_TEST_TASK_LOCK_DELAY_US=50 ...
ninja ves_mecaapi_cal_test
```

| Parameter | CMake variable | Default |
|---|---|---|
| RCU pages per block | `CAL_TEST_PAGE_COUNT` | 5 |
| Reader threads | `CAL_TEST_THREAD_COUNT` | 4 |
| Calibration writes | `CAL_TEST_WRITE_COUNT` | 10000 |
| Writer loop delay | `CAL_TEST_MAIN_LOOP_DELAY_US` | 100 |
| Every Nth write atomic | `CAL_TEST_ATOMIC_CAL` | 10 |
| Reader loop delay | `CAL_TEST_TASK_LOOP_DELAY_US` | 100 |
| Reader handle hold time | `CAL_TEST_TASK_LOCK_DELAY_US` | 0 |
| Payload filler size | `CAL_TEST_DATA_SIZE` | 8 |

It runs two phases and returns non-zero on any error:

- **RCU level phase** - one writer thread and `CAL_TEST_THREAD_COUNT` readers directly on the `Rcu`. This phase
  respects the single writer precondition, so it is the phase to run under ThreadSanitizer (`--rcu-only`).
- **Calibration block phase** - the same load through `CalibrationBlock`, `WriteCalibrationBlockBytes` and
  `GetHandle()`, including atomic transactions and the update callback. It also reports the lock time histogram.

Checks in both phases: every observed payload must satisfy its checksum (no torn read, no partially published
transaction), reads must be monotonic, a nested handle must observe the same page, and the last written value must
become visible.

### Results

Release build, x86_64, default parameters, against the figures CAL_RCU.md publishes for the three-page pattern:

| | CAL_RCU.md (MacBook M3) | CAL_RCU.md (Raspberry Pi 5) | This implementation (x86_64) |
|---|---|---|---|
| Writes | 10000 | 10000 | 10000 |
| Reads | 53441 | 52003 | 39918 |
| Changes observed | 37082 (**69.4 %**) | 38380 (**73.8 %**) | 39183 (**98.2 %**) |
| Writes pending at the end | 136 | 1 | **0** |
| Average lock time | 0.13 us | 0.36 us | 0.06 us |
| Maximum lock time | 25.71 us | 8.72 us | 8.68 us |
| Errors | 0 | 0 | 0 |

The share of reads that observe a change is the metric that matters here, and it is not hardware dependent: it
measures how quickly a published update reaches the readers. Lock times are not comparable across these rows
because the hardware differs.

RCU level phase, same run: 10000 publishes at 5.06 M/s, 1 deferred attempt (0.01 %), 0 errors. Under
ThreadSanitizer with `--rcu-only`: 0 reports.

### Note on the calibration block phase and ThreadSanitizer

The block phase drives calibration writes from the application thread, while the cyclic housekeeping timer calls
`TryPublishPendingCalibrationBlocks()` on the ComAb context. That is two writers, which neither `Rcu::TryPublish()`
nor the `has_pending_update_` / `consistency_hold_` flags of `CalibrationBlock` support. ThreadSanitizer reports
races on that bookkeeping in this phase.

This is a property of the existing API rather than of this change, and it does not occur in a real system, where
calibration writes arrive from the daemon on the same ComAb context that runs the housekeeping timer. It is
nevertheless an undocumented constraint - a calibration block may only be written from the ComAb context - and it
is closely related to open issue 1 of CAL_RCU.md. Making the two flags atomic would remove the flag races but not
the underlying one, since `TryPublish()` copies the writer scratch page.
