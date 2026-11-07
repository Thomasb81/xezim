//! The SystemC host thread (SC-top mode).

use std::ffi::c_char;
use std::sync::Arc;

use super::build::build_and_load;
use super::link::ScLink;

/// Body of the SystemC host thread (SC top). Compiles + loads the cosim .so,
/// publishes the shim, and blocks in `xsc_cosim_main` until `sc_main`
/// returns.
pub fn run_sc_top_thread(link: Arc<ScLink>) {
    let opts = match super::options::sc_options() {
        Some(o) => o,
        None => {
            link.mark_sc_done();
            return;
        }
    };

    let Some(tick_s) = link.wait_tick_s() else {
        // SV side never came up; nothing to co-simulate with.
        link.mark_sc_done();
        return;
    };

    // The SV worker knows the top module's ports only after its own
    // elaboration; wait for it to publish the auto-derived wrapper (if any)
    // before compiling the model so the generated files are materialised.
    let bundle = link.await_wrapper().unwrap_or_default();

    match build_and_load(&opts, &bundle) {
        Ok(api) => {
            let shim = Arc::new(api);
            link.publish_shim(shim.clone());
            let mut argv = [b"xezim\0".as_ptr() as *mut c_char];
            let rc = unsafe {
                (shim.cosim_main)(
                    1,
                    argv.as_mut_ptr() as *mut *mut c_char,
                    std::ptr::null(),
                    tick_s,
                )
            };
            if rc != 0 {
                eprintln!("[xezim-sc] xsc_cosim_main returned {rc}");
            }
        }
        Err(e) => {
            eprintln!("Error: {e}");
        }
    }
    link.mark_sc_done();
}