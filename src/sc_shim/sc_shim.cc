// xezim SystemC co-simulation shim.
//
// One translation unit serves BOTH integration modes:
//
//   SV top (--sc-top sv, the default):  xezim is the time master. The user's
//     SystemC model is compiled together with this file into a model .so that
//     exports xsc_model_init(); $sc_step / $sc_export_sig / $sc_import_sig
//     drive it synchronously from the SystemVerilog testbench.
//
//   SC top (--sc-top sc):  SystemC is the master. The user's sc_main() TU is
//     compiled together with this file into a cosim .so exporting
//     xsc_cosim_main(); xezim runs its own kernel on a worker thread that is
//     stepped by the spawned "xezim_host" SC thread through the link below.
//
// Build requirements: SystemC 3.x (IEEE 1666-2011), C++17.
// See docs/systemc-cosim.md for the full protocol description.

#include "sc_api.h"

#include <cmath>
#include <condition_variable>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <string>
#include <vector>

#include <systemc>

extern "C" int sc_main(int argc, char* argv[]);        // provided by the user TU (SC top)
extern "C" void xsc_model_build();                     // provided by the user TU (SV top)

namespace {

// ── Registry ────────────────────────────────────────────────────────────────
//
// Ordered list of boundary signals bound with xsc_bind(). Index == position
// in the registry; the Rust side resolves names to indices once at handshake
// and then addresses signals by index (stable, no string traffic per slice).

struct XscSig {
    std::string name;
    int width;  // bits
    std::function<void(const std::uint64_t*, int)> write;  // SV -> SC
    std::function<void(std::uint64_t*, int)> read;         // SC -> SV
};

std::mutex g_reg_mu;
std::vector<XscSig> g_reg;

// Export staging (SV thread writes, SC thread applies). One slot per
// registry entry, sized ceil(width/64) words.
std::vector<std::vector<std::uint64_t>> g_exp_stage;
std::vector<std::vector<std::uint64_t>> g_imp_stage;
// Per-slot dirty flags: which exports were staged since the last apply.
std::vector<char> g_exp_dirty;

int words_for(int width) { return (width + 63) / 64; }

}  // namespace

namespace xezim {
namespace detail {
void reg_signal(const char* name, int width,
                std::function<void(const std::uint64_t*, int)>&& writer,
                std::function<void(std::uint64_t*, int)>&& reader) {
    std::lock_guard<std::mutex> lk(g_reg_mu);
    for (const auto& s : g_reg)
        if (s.name == name) {
            std::fprintf(stderr, "[xezim-sc] xsc_bind: duplicate name '%s'\n", name);
            return;
        }
    XscSig s;
    s.name = name;
    s.width = width;
    s.write = std::move(writer);
    s.read = std::move(reader);
    g_reg.push_back(std::move(s));
    g_exp_stage.emplace_back(words_for(width), 0);
    g_exp_dirty.push_back(0);
    g_imp_stage.emplace_back(words_for(width), 0);
}
}  // namespace detail
}  // namespace xezim

namespace {

// ── Time mapping ────────────────────────────────────────────────────────────
//
// The SystemC time resolution is set to exactly one xezim tick, so 1 xezim
// tick == 1 sc_time value unit and every boundary crossing is an exact
// integer (sc_time::from_value(n) == n ticks).

bool set_resolution_from_tick(double tick_s) {
    static const struct { double secs; sc_core::sc_time_unit unit; const char* name; } units[] = {
        {1e-15, sc_core::SC_FS, "FS"}, {1e-12, sc_core::SC_PS, "PS"},
        {1e-9,  sc_core::SC_NS, "NS"}, {1e-6,  sc_core::SC_US, "US"},
        {1e-3,  sc_core::SC_MS, "MS"}, {1e+0,  sc_core::SC_SEC, "SEC"},
    };
    for (const auto& u : units) {
        for (double m : {1.0, 10.0, 100.0}) {
            if (std::fabs(m * u.secs - tick_s) <= tick_s * 1e-9) {
                sc_core::sc_set_time_resolution(m, u.unit);
                return true;
            }
        }
    }
    std::fprintf(stderr,
                 "[xezim-sc] ERROR: tick length %gs cannot be expressed as a SystemC time "
                 "resolution (1/10/100 x unit, floor 1 fs). Check the design timescales.\n",
                 tick_s);
    return false;
}

// ── SV-top link / SV-SC shared rendezvous state ─────────────────────────────
//
// Only used by --sc-top sc, but declared unconditionally so both sides of the
// Rust/C++ boundary can reference the same ops.

std::mutex g_link_mu;
std::condition_variable g_link_cv;
// Round-token handshake: SV posts round N (g_exp_posted = N) after staging
// its exports; SC consumes everything staged (g_imp_done = g_exp_posted)
// after running its slice. Comparing tokens instead of boolean flags keeps
// the two sides strictly serialized: an SC slice can never run against
// not-yet-staged exports, and the SV kernel can never run twice per slice.
std::uint64_t g_exp_posted = 0;
std::uint64_t g_imp_done = 0;
bool g_sv_finished = false;
bool g_sc_done = false;
std::uint64_t g_cap = 0;      // SystemC master's current time (ticks)
std::uint64_t g_quantum = 1000;

const XscHostCb* g_cb = nullptr;

void apply_exports() {
    // Staged exports -> registry signals. The registry itself is immutable
    // after elaboration; the staging handoff is guarded by g_link_mu. Only
    // slots explicitly staged since the last apply are written — an import
    // slot's staging buffer must never clobber the value the SC design drove.
    std::lock_guard<std::mutex> lk(g_link_mu);
    for (size_t i = 0; i < g_reg.size(); ++i) {
        if (g_exp_dirty[i]) {
            g_exp_dirty[i] = 0;
            g_reg[i].write(g_exp_stage[i].data(), (int)g_exp_stage[i].size());
        }
    }
}

void capture_imports() {
    for (size_t i = 0; i < g_reg.size(); ++i)
        g_reg[i].read(g_imp_stage[i].data(), (int)g_imp_stage[i].size());
}

// The SC-side slice step (SC top). Rendezvous protocol per slice:
//   1. tell the SV kernel our current time (cap) and wait for its exports
//   2. apply exports; the caller then sc_start()s up to the next activity
//   3. the caller captures imports and closes the round (close_slice)
int sv_step_exchange(void*, std::uint64_t cap, std::uint64_t quantum) {
    std::unique_lock<std::mutex> lk(g_link_mu);
    g_cap = cap;
    g_quantum = quantum;
    g_link_cv.wait(lk, [] { return g_exp_posted > g_imp_done || g_sv_finished; });
    if (g_sv_finished) return 1;
    lk.unlock();
    apply_exports();
    return 0;
}

// Close the round opened by sv_step_exchange: record that every staged
// export round is consumed and wake the SV kernel with the fresh imports.
void close_slice() {
    std::lock_guard<std::mutex> lk(g_link_mu);
    g_imp_done = g_exp_posted;
    g_link_cv.notify_all();
}


// The slice loop. Runs on sc_main()'s own thread — the only context from
// which sc_start() is legal — so it must be called from the user's sc_main()
// INSTEAD of sc_start(). Each iteration rendezvouses with the xezim kernel
// (exports in), advances SystemC to its next activity bounded by the quantum,
// and hands the captured imports back.
void slice_loop() {
    bool first = true;
    while (true) {
        const std::uint64_t now = sc_core::sc_time_stamp().value();
        if (g_cb->sv_step_exchange(g_cb->ctx, now, g_quantum)) {
            break;
        }
        // Advance SystemC to its next pending activity, bounded by the
        // quantum. A quiescent kernel still advances its clock so the SV
        // side can make progress against a growing cap.
        // sc_time_to_pending_activity returns the DELTA to the next activity
        // (a huge sentinel value when the kernel is quiescent).
        try {
            if (first) {
                first = false;
                // Elaborate + initialize now: the first sc_start() call runs
                // the kernel's initialization deltas, so the pending-activity
                // query below sees real scheduled events. A not-yet-started
                // kernel reports quiescent, which would run the entire
                // quantum in one slice before the master even gets going.
                sc_core::sc_start(sc_core::SC_ZERO_TIME, sc_core::SC_RUN_TO_TIME);
            }
            const sc_core::sc_time act = sc_core::sc_time_to_pending_activity(
                sc_core::sc_get_curr_simcontext());
            std::uint64_t step = g_quantum;
            const std::uint64_t a = act.value();
            if (a < step) step = a;
            // step == 0 (activity pending at the current time) maps to
            // sc_start(SC_ZERO_TIME): drains the pending delta cycles only.
            sc_core::sc_start(sc_core::sc_time::from_value((double)step),
                              sc_core::SC_RUN_TO_TIME);
        } catch (const std::exception& e) {
            std::fprintf(stderr,
                         "[xezim-sc] slice sc_start failed: %s\n"
                         "  (with --sc-top sc, sc_main() must call xsc_cosim_run() "
                         "instead of sc_start())\n",
                         e.what());
            std::lock_guard<std::mutex> lk(g_link_mu);
            g_sc_done = true;
            g_link_cv.notify_all();
            return;
        }
        capture_imports();
        close_slice();
    }
    // SV side finished ($finish / max-time / both kernels quiescent): end
    // the kernel so sc_main can return. Between-runs sc_stop() from
    // sc_main's thread is legal.
    try {
        if (sc_core::sc_end_of_simulation_invoked() == false &&
            sc_core::sc_start_of_simulation_invoked()) {
            sc_core::sc_stop();
        }
    } catch (...) {
    }
}

}  // namespace

// ── C ABI: model (SV top) + shared signal access ────────────────────────────

extern "C" {

// 1 when the SystemC kernel can still produce future activity, 0 when it is
// quiescent. A not-yet-started kernel reports "pending" so the SV side
// keeps exchanging through the first slices (elaboration happens at the
// first sc_start inside the slice loop).
int xsc_has_pending(void) {
    try {
        if (!sc_core::sc_start_of_simulation_invoked()) return 1;
        const sc_core::sc_time act = sc_core::sc_time_to_pending_activity(
            sc_core::sc_get_curr_simcontext());
        // Quiescent reports a max-time sentinel.
        return act.value() < (std::numeric_limits<std::uint64_t>::max() >> 2) ? 1 : 0;
    } catch (...) {
        return 0;
    }
}

int xsc_model_init(double tick_s) {
    if (!set_resolution_from_tick(tick_s)) return 2;
    try {
        xsc_model_build();
    } catch (const std::exception& e) {
        std::fprintf(stderr, "[xezim-sc] model build failed: %s\n", e.what());
        return 3;
    }
    return 0;
}

void xsc_model_finish(void) {
    // Idempotent: ends simulation if the kernel was started and never stopped.
    try {
        if (sc_core::sc_is_running()) sc_core::sc_stop();
    } catch (...) {
    }
}

int xsc_signal_count(void) {
    std::lock_guard<std::mutex> lk(g_reg_mu);
    return (int)g_reg.size();
}

const char* xsc_signal_name(int idx) {
    std::lock_guard<std::mutex> lk(g_reg_mu);
    if (idx < 0 || (size_t)idx >= g_reg.size()) return nullptr;
    return g_reg[idx].name.c_str();
}

int xsc_signal_width(int idx) {
    std::lock_guard<std::mutex> lk(g_reg_mu);
    if (idx < 0 || (size_t)idx >= g_reg.size()) return -1;
    return g_reg[idx].width;
}

int xsc_find_signal(const char* name) {
    std::lock_guard<std::mutex> lk(g_reg_mu);
    for (size_t i = 0; i < g_reg.size(); ++i)
        if (g_reg[i].name == name) return (int)i;
    return -1;
}

void xsc_write_export(int idx, const std::uint64_t* w, int n) {
    std::lock_guard<std::mutex> lk(g_link_mu);
    if (idx < 0 || (size_t)idx >= g_exp_stage.size()) return;
    auto& dst = g_exp_stage[idx];
    for (int j = 0; j < n && j < (int)dst.size(); ++j) dst[j] = w[j];
    g_exp_dirty[idx] = 1;
}

void xsc_read_import(int idx, std::uint64_t* w, int n) {
    if (idx < 0 || (size_t)idx >= g_reg.size()) return;
    g_reg[idx].read(w, n);
}

void xsc_step(std::uint64_t ticks) {
    if (ticks == 0) return;
    try {
        apply_exports();
        // First call triggers elaborate()/initialize() internally; the run
        // always advances exactly `ticks` (SC_RUN_TO_TIME), so the SV side's
        // view of SystemC time stays exact.
        sc_core::sc_start(sc_core::sc_time::from_value((double)ticks),
                          sc_core::SC_RUN_TO_TIME);
    } catch (const std::exception& e) {
        std::fprintf(stderr, "[xezim-sc] xsc_step failed: %s\n", e.what());
    }
}

std::uint64_t xsc_time_now(void) {
    try {
        return sc_core::sc_time_stamp().value();
    } catch (...) {
        return 0;
    }
}

// ── C ABI: SC-top entry ─────────────────────────────────────────────────────

int xsc_cosim_main(int argc, char** argv, const XscHostCb* cb, double tick_s) {
    if (!set_resolution_from_tick(tick_s)) return 2;
    // cb may carry a custom per-slice hook; NULL selects the built-in
    // rendezvous implemented by sv_step_exchange() above.
    static XscHostCb default_cb;
    if (!cb || !cb->sv_step_exchange) {
        default_cb.ctx = nullptr;
        default_cb.sv_step_exchange = [](void* ctx, std::uint64_t cap,
                                         std::uint64_t quantum) {
            return sv_step_exchange(ctx, cap, quantum);
        };
        g_cb = &default_cb;
    } else {
        g_cb = cb;
    }
    try {
        // sc_main() runs here, on this thread. For sliced co-simulation the
        // user's sc_main() must NOT call sc_start(): it builds modules, calls
        // xsc_bind(), then calls xsc_cosim_run() (the slice loop), and the
        // sc_start() calls inside that loop are legal because they execute
        // on sc_main's own thread. After sc_main returns we report done.
        int rc = sc_core::sc_elab_and_sim(argc, argv);
        {
            std::lock_guard<std::mutex> lk(g_link_mu);
            g_sc_done = true;
        }
        g_link_cv.notify_all();
        return rc;
    } catch (const std::exception& e) {
        std::fprintf(stderr, "[xezim-sc] SystemC simulation failed: %s\n", e.what());
        std::lock_guard<std::mutex> lk(g_link_mu);
        g_sc_done = true;
        g_link_cv.notify_all();
        return 5;
    }
}

void xsc_cosim_run(void) { slice_loop(); }

// ── C ABI: SV-side link ops (SC top) ────────────────────────────────────────

// Called by the xezim event loop at a co-sim boundary: publishes the staged
// exports and blocks until the SystemC slice has produced its imports.
// Returns 0 (ok) or 1 (SystemC side finished — stop co-simulation).
int xsc_sv_wait_slice(void) {
    std::unique_lock<std::mutex> lk(g_link_mu);
    const std::uint64_t my = g_exp_posted + 1;
    g_exp_posted = my;
    lk.unlock();
    g_link_cv.notify_all();
    lk.lock();
    g_link_cv.wait(lk, [&] { return g_imp_done >= my || g_sc_done; });
    if (g_sc_done && g_imp_done < my) return 1;
    return 0;
}

const std::uint64_t* xsc_sv_import_ptr(int idx) {
    if (idx < 0 || (size_t)idx >= g_imp_stage.size()) return nullptr;
    return g_imp_stage[idx].data();
}

void xsc_sv_set_finished(void) {
    {
        std::lock_guard<std::mutex> lk(g_link_mu);
        g_sv_finished = true;
    }
    g_link_cv.notify_all();
}

// SystemC master's current time in ticks, as of the last completed slice.
std::uint64_t xsc_sv_cap(void) {
    std::lock_guard<std::mutex> lk(g_link_mu);
    return g_cap;
}

}  // extern "C"
