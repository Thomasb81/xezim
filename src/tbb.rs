//! Thin Rust wrapper around the TBB shim defined in `tbb_shim.cpp`.
//!
//! Active only with `--features tbb`. Without the feature the public
//! `parallel_for_partitions` is a stub that runs partitions sequentially
//! on the calling thread, so call sites don't need feature-gating.

#[cfg(feature = "tbb")]
mod ffi {
    use std::ffi::c_void;
    extern "C" {
        pub fn xezim_tbb_parallel_for_partitions(
            user: *mut c_void,
            n_partitions: usize,
            cb: extern "C" fn(*mut c_void, usize),
            grain: usize,
        );
        pub fn xezim_tbb_set_threads(n: usize) -> *mut c_void;
        pub fn xezim_tbb_drop_threads(handle: *mut c_void);
    }
}

/// Run `f(p)` for each `p` in `0..n_partitions` using TBB's work-stealing
/// scheduler when feature `tbb` is enabled, sequentially otherwise.
///
/// `f` must be `Send + Sync` because TBB may invoke it concurrently
/// from worker threads. The closure receives only the partition index;
/// shared state is captured by reference.
pub fn parallel_for_partitions<F>(n_partitions: usize, grain: usize, f: F)
where
    F: Fn(usize) + Send + Sync,
{
    #[cfg(feature = "tbb")]
    {
        use std::ffi::c_void;
        // Trampoline: cast the user-data pointer back to the Rust
        // closure and invoke it. Safety: the closure is alive for
        // the duration of the C call (we block on parallel_for).
        extern "C" fn trampoline<F: Fn(usize) + Send + Sync>(
            user: *mut c_void,
            p: usize,
        ) {
            // SAFETY: `user` is a valid `*const F` for the duration
            // of the parallel_for call (see caller).
            let f = unsafe { &*(user as *const F) };
            f(p);
        }
        unsafe {
            ffi::xezim_tbb_parallel_for_partitions(
                &f as *const F as *mut c_void,
                n_partitions,
                trampoline::<F>,
                grain,
            );
        }
    }
    #[cfg(not(feature = "tbb"))]
    {
        let _ = grain;
        for p in 0..n_partitions {
            f(p);
        }
    }
}

/// RAII handle that pins TBB's worker count for its lifetime. Drop to
/// release. Without the `tbb` feature this is a no-op stub.
pub struct ThreadGuard {
    #[cfg(feature = "tbb")]
    handle: *mut std::ffi::c_void,
}

impl ThreadGuard {
    pub fn new(n: usize) -> Self {
        #[cfg(feature = "tbb")]
        {
            let handle = unsafe { ffi::xezim_tbb_set_threads(n) };
            return Self { handle };
        }
        #[cfg(not(feature = "tbb"))]
        {
            let _ = n;
            Self {}
        }
    }
}

impl Drop for ThreadGuard {
    fn drop(&mut self) {
        #[cfg(feature = "tbb")]
        unsafe {
            ffi::xezim_tbb_drop_threads(self.handle);
        }
    }
}

/// Returns true iff this build was compiled with `--features tbb`.
pub const fn is_available() -> bool {
    cfg!(feature = "tbb")
}
