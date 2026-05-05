// Intel TBB shim for xezim's parallel edge-block dispatcher.
//
// Built only when `--features tbb` is enabled (gated by build.rs).
// Exposes a single C ABI entry point `xezim_tbb_parallel_for_partitions`
// that iterates [0, n_partitions) using TBB's work-stealing scheduler
// and calls the Rust callback for each partition index.
//
// Why a tiny shim and not a full TBB Rust binding crate: oneapi-rs and
// tbb-rs are heavyweight + brittle across LLVM/TBB ABI versions. A
// 30-line shim using TBB's stable C++ API is simpler to maintain and
// builds cleanly against the system libtbb-dev (apt: 2021.11.0).

#include <oneapi/tbb/parallel_for.h>
#include <oneapi/tbb/blocked_range.h>
#include <oneapi/tbb/global_control.h>
#include <cstddef>

extern "C" {

// Callback signature: `extern "C" fn(user: *mut c_void, partition_idx: usize)`.
typedef void (*xezim_tbb_partition_fn)(void* user, std::size_t partition_idx);

// Run `fn(user, p)` for p in [0, n_partitions) in parallel using TBB.
// Each partition is one task; TBB's work-stealing scheduler
// dynamically rebalances if some take longer than others. `grain` is
// the minimum number of partitions per task — 1 means "every partition
// is its own task" (max parallelism, more scheduler overhead).
void xezim_tbb_parallel_for_partitions(
    void* user,
    std::size_t n_partitions,
    xezim_tbb_partition_fn fn,
    std::size_t grain
) {
    if (n_partitions == 0) {
        return;
    }
    oneapi::tbb::parallel_for(
        oneapi::tbb::blocked_range<std::size_t>(0, n_partitions, grain ? grain : 1),
        [user, fn](const oneapi::tbb::blocked_range<std::size_t>& r) {
            for (std::size_t p = r.begin(); p < r.end(); ++p) {
                fn(user, p);
            }
        }
    );
}

// Pin the TBB scheduler to a fixed thread count for the lifetime of
// the returned pointer (an opaque global_control object). Caller must
// pass it back to `xezim_tbb_drop_threads` to release.
void* xezim_tbb_set_threads(std::size_t n_threads) {
    if (n_threads == 0) {
        return nullptr;
    }
    return new oneapi::tbb::global_control(
        oneapi::tbb::global_control::max_allowed_parallelism, n_threads
    );
}

void xezim_tbb_drop_threads(void* handle) {
    if (handle) {
        delete static_cast<oneapi::tbb::global_control*>(handle);
    }
}

} // extern "C"
