//! SystemC co-simulation (SystemC ↔ SystemVerilog).
//!
//! Runtime-optional: without any `--sc-*` flag nothing in this module runs and
//! the simulator behaves exactly as before (no build-time dependency on
//! SystemC; the C++ side is compiled and `dlopen`-ed lazily).
//!
//! Two integration modes; the master (time owner) is inferred, not picked:
//!
//! * **SV top** (a passive `xsc_model_build`): xezim is the time master. The
//!   user's SystemC model sources are compiled together with
//!   `src/sc_shim/sc_shim.cc` into a model shared object exposing
//!   `xsc_model_init()`. The SV testbench drives it explicitly with `$sc_step`,
//!   `$sc_export_sig`, `$sc_import_sig` and `$sc_finish`.
//!
//! * **SC top** (a strong `sc_main`): SystemC is the time master. The user's
//!   `sc_main()` sources are compiled together with the shim into a cosim
//!   shared object exposing `xsc_cosim_main()`; xezim runs its kernel on a
//!   worker thread that rendezvouses with the spawned `xezim_host` SystemC
//!   thread once per slice (bounded by `XEZIM_SC_QUANTUM`). SystemVerilog
//!   signals are attached by name through the auto-derived wrapper.
//!
//! Auto-derived wrappers: like the reference simulators (and Verilator's
//! `Vtop`), xezim *generates* the interop boundary instead of asking the user
//! to hand-write `xsc_bind`/map calls. See [`wrapper`].
//!
//! Time mapping: the SystemC time resolution is set to exactly one xezim
//! tick, so one `sc_time` value unit == one xezim tick everywhere at the
//! boundary.
//!
//! Layout: this crate module is split into focused submodules.
//!
//! * [`options`] — CLI-facing configuration (`ScOptions`, `ScTop`, `ScDir`,
//!   entry-point inference, shim path resolution).
//! * [`wrapper`] — auto-derivation of the SV↔SC boundary
//!   (`TopPort`, `WrapperBundle`, `generate_wrapper`, the `XezTop` / SV-master
//!   renderers).
//! * [`link`] — `ScLink` (state shared between the two kernels) and the
//!   `ScShimApi` shared-object ABI table.
//! * [`build`] — `build_and_load`: compile the user sources + shim +
//!   generated wrapper into a model `.so` and `dlopen` it.
//! * [`bridge`] — per-`Simulator` runtime state (`ScBridge`, mapping rows,
//!   trace mirrors).
//! * [`host`] — the SystemC host thread (`run_sc_top_thread`).

pub mod bridge;
pub mod build;
pub mod host;
pub mod link;
pub mod options;
pub mod placeholder;
pub mod wrapper;

pub use bridge::{ScBridge, ScMapRow, ScTraceRow};
pub use build::build_and_load;
pub use host::run_sc_top_thread;
pub use link::{store_pending_link, take_pending_link, ScLink, ScShimApi, XscHostCb};
pub use options::{
    infer_master, infer_sc_top, quantum_from_env, sc_module_ports, sc_options, sc_top,
    set_sc_options, shim_include_dir, shim_source, ScDir, ScOptions, ScPort, ScTop,
};
pub use placeholder::synthesize_placeholder;
pub use wrapper::{
    generate_wrapper, set_top_ports, top_ports, ScPortDir, TopPort, WrapperBundle,
};