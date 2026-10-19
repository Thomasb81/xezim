// xezim SystemC co-simulation — user-facing API.
//
// Include this header from the C++ translation unit that builds the SystemC
// side of a co-simulation model and bind every SystemC signal that crosses
// the boundary with xsc_bind(). Bound signals form the co-simulation
// REGISTRY; the SystemVerilog side references them by the same name
// ($sc_export_sig / $sc_import_sig for a SystemVerilog top; the auto-derived
// wrapper binds a SystemC top by name).
//
//   SC_TOP (SystemVerilog DUT):          SV_TOP (SystemC DUT):
//   int sc_main(int, char**) {           extern "C" void xsc_model_build() {
//     ...                                  MyScDut dut("dut");
//     xezim_sc_model u_dut("dut");         xsc_bind("stim.valid", dut.valid);
//     xsc_bind("dut.q", dut.q);            ...
//     xsc_bind("tb.clk", clk);           }
//     xsc_cosim_run();                   // NOT sc_start(): xezim drives the
//     return 0;                          // simulation in slices from inside
//   }                                    // xsc_cosim_run()
//
// Two-state semantics: sc_lv 'X'/'Z' bits read back as 0 on the SystemVerilog
// side (SystemVerilog X/Z written into an sc_lv become '0').
#ifndef XEZIM_SC_API_H
#define XEZIM_SC_API_H

#include <cstdint>
#include <cstdint>
#include <functional>
#include <string>
#include <systemc>

namespace xezim {

namespace detail {
// Registry entry registration (implemented in sc_shim.cc). Not part of the
// user API — use the xsc_bind() overloads below.
void reg_signal(const char* name, int width,
                std::function<void(const std::uint64_t*, int)>&& writer,
                std::function<void(std::uint64_t*, int)>&& reader);
}  // namespace detail

/// Bind an sc_signal<bool> under `name`. Width 1.
inline void xsc_bind(const char* name, sc_core::sc_signal<bool>& sig) {
    detail::reg_signal(
        name, 1,
        [&sig](const std::uint64_t* w, int) { sig.write(w[0] != 0); },
        [&sig](std::uint64_t* w, int) { w[0] = sig.read() ? 1 : 0; });
}

/// Bind an sc_signal of any small integer type (unsigned, int, std::uint64_t,
/// ...) under `name`. The boundary width is 64 bits; values are sign/zero
/// extended by the caller's type. (`bool` has its own overload above.)
template <typename T,
          typename = std::enable_if_t<std::is_integral_v<T> && !std::is_same_v<T, bool>>>
inline void xsc_bind(const char* name, sc_core::sc_signal<T>& sig) {
    detail::reg_signal(
        name, 64,
        [&sig](const std::uint64_t* w, int) { sig.write(static_cast<T>(w[0])); },
        [&sig](std::uint64_t* w, int) { w[0] = static_cast<std::uint64_t>(sig.read()); });
}

/// Bind an sc_signal<sc_dt::sc_lv<W>> under `name` (1 <= W <= 4096).
/// Bit i of the sc_lv maps to bit i of the co-simulation word stream
/// (word j holds bits [64j, 64j+63], LSB first — SystemVerilog order).
template <int W>
inline void xsc_bind(const char* name, sc_core::sc_signal<sc_dt::sc_lv<W>>& sig) {
    static_assert(W >= 1 && W <= 4096, "xsc_bind: width must be 1..4096");
    constexpr int NW = (W + 63) / 64;
    detail::reg_signal(
        name, W,
        [&sig](const std::uint64_t* w, int) {
            sc_dt::sc_lv<W> v;
            for (int b = 0; b < W; ++b)
                v.set_bit(b, (w[b / 64] >> (b % 64)) & 1 ? sc_dt::Log_1 : sc_dt::Log_0);
            sig.write(v);
        },
        [&sig](std::uint64_t* w, int) {
            for (int j = 0; j < NW; ++j) w[j] = 0;
            const sc_dt::sc_lv<W> v = sig.read();
            for (int b = 0; b < W; ++b)
                if (v.get_bit(b) == sc_dt::Log_1) w[b / 64] |= std::uint64_t(1) << (b % 64);
        });
}

/// Bind an `sc_signal<sc_dt::sc_uint<W>>` under `name`. `sc_uint<N>` is a
/// 2-state unsigned word type (not a C++ integral, so the generic `T` overload
/// above does not match it); it is exactly uintN_t on the value bound side.
/// Bit i maps to bit i of the 64-bit crossing word (LSB first).
template <int W>
inline void xsc_bind(const char* name, sc_core::sc_signal<sc_dt::sc_uint<W>>& sig) {
    static_assert(W >= 1 && W <= 64, "xsc_bind: sc_uint width must be 1..64");
    detail::reg_signal(
        name, W,
        [&sig](const std::uint64_t* w, int) {
            sig.write(sc_dt::sc_uint<W>(w[0] & ((W >= 64) ? ~std::uint64_t(0) : ((std::uint64_t(1) << W) - 1))));
        },
        [&sig](std::uint64_t* w, int) { w[0] = static_cast<std::uint64_t>(sig.read()); });
}

/// Bind an `sc_signal<sc_dt::sc_bv<W>>` under `name`. 2-state equivalent of
/// the `sc_lv` overload.
template <int W>
inline void xsc_bind(const char* name, sc_core::sc_signal<sc_dt::sc_bv<W>>& sig) {
    static_assert(W >= 1 && W <= 4096, "xsc_bind: sc_bv width must be 1..4096");
    constexpr int NW = (W + 63) / 64;
    detail::reg_signal(
        name, W,
        [&sig](const std::uint64_t* w, int) {
            sc_dt::sc_bv<W> v;
            for (int b = 0; b < W; ++b)
                v[b] = ((w[b / 64] >> (b % 64)) & 1) ? sc_dt::SC_LOGIC_1 : sc_dt::SC_LOGIC_0;
            sig.write(v);
        },
        [&sig](std::uint64_t* w, int) {
            for (int j = 0; j < NW; ++j) w[j] = 0;
            const sc_dt::sc_bv<W> v = sig.read();
            for (int b = 0; b < W; ++b)
                if (v[b] == sc_dt::SC_LOGIC_1) w[b / 64] |= std::uint64_t(1) << (b % 64);
        });
}

// ── Module parameter registry accessors (parameter passing) ─────────────────
// Implemented in sc_shim.cc. These read the shared parameter table filled from
// the boundary (SV override at the SC-module instantiation site, or an SC
// call into the SV override mechanism). See the free `xsc_get_*_param` /
// `sc_get_*_param` shorthands below.
int param_get_int(const char* name, int fallback, int* found);
double param_get_real(const char* name, double fallback, int* found);
const char* param_get_string(const char* name, const char* fallback, int* found);

}  // namespace xezim

using xezim::xsc_bind;

// ── SC-top host callback ABI ────────────────────────────────────────────────
// Passed to xsc_cosim_main() by the xezim binary; the shim calls it once per
// co-simulation slice from xsc_cosim_run()'s slice loop, which runs on
// sc_main()'s own thread. Returns 0 to continue, 1 when the SystemVerilog
// kernel has finished ($finish / SystemC must stop).
extern "C" struct XscHostCb {
    void* ctx;
    int (*sv_step_exchange)(void* ctx, std::uint64_t cap, std::uint64_t quantum);
};

// Run the co-simulation slice loop. Call this from sc_main() INSTEAD of
// sc_start(): xezim drives the SystemC kernel one slice at a time and each
// slice rendezvouses with the SystemVerilog kernel through the host callback.
// The sc_start() calls inside execute on sc_main's own thread - the only
// context the standard allows them in.
extern "C" void xsc_cosim_run(void);

// 1 when the SystemC kernel can still produce future activity, 0 when it is
// quiescent (no scheduled events anywhere). xezim uses this to decide that
// the co-simulation is over: when both kernels are quiescent there is
// nothing left that could resume parked SystemVerilog processes.
extern "C" int xsc_has_pending(void);

// ── Module parameter passing ────────────────────────────────────────────────
//
// IEEE 1666-2011 §4.7 `/` the reference-simulator SystemC examples expose a
// generic-parameter list across the boundary: a SystemC module can read
// parameters that the SystemVerilog side overrides at the instantiation site
// (`ringbuf #(.counter_size(4)) u(...)` -> `sc_get_int_param("counter_size"
// , &status)`). xezim mirrors the same free functions so a pure-SystemC module
// is source-compatible. Under the hood the parameters live in the shim registry
// and are filled from either side (SV -> SC today; SC -> SV is the same
// registry mirrored onto SV parameter overrides).
//
// The `*_status`/`ret` out-params (and the trailing dummy args some overloads
// take) match the reference-simulator signatures so existing models compile
// unchanged. A parameter that is absent reads as its fallback and sets the
// status to 0 ("not found").

/// Read an integer parameter; `*found` = 1 if present, else 0. `fallback` is
/// returned when the parameter is absent.
inline int xsc_get_int_param(const char* name, int fallback, int* found) {
    return xezim::param_get_int(name, fallback, found);
}
/// Read a real (double) parameter; `*found` = 1 if present, else 0.
inline double xsc_get_real_param(const char* name, double fallback, int* found) {
    return xezim::param_get_real(name, fallback, found);
}
/// Read a string parameter; `*found` = 1 if present, else 0.
inline const char* xsc_get_string_param(const char* name, const char* fallback, int* found) {
    return xezim::param_get_string(name, fallback, found);
}

// IEEE 1666 generic-parameter spelling (reference-simulator source-compatible).
// Signatures match a reference simulator's bundled SystemC `sc_module` parameter accessors
// (`sc_get_*_param`), so a reference `param_passing` model compiles unchanged:
//   int         sc_get_int_param(name, int* status)
//   double      sc_get_real_param(name, int* status)      // NOTE: status is int*, not double*
//   std::string sc_get_string_param(name, char fmt = 'a', int* status)
//   int         sc_get_param(name, int& / double& / std::string&, char fmt = 'a')
inline int sc_get_int_param(const char* name, int* status = 0) {
    int f = 0; int v = xsc_get_int_param(name, 0, &f); if (status) *status = f; return v;
}
inline double sc_get_real_param(const char* name, int* status = 0) {
    int f = 0; double v = xsc_get_real_param(name, 0.0, &f); if (status) *status = f; return v;
}
/// Read a string parameter. `format_char` selects the representation ('a' = ASCII
/// quote-stripped, 'd', 'h', 'b', 'o') like the reference; the SV side stores the
/// value as a string, so 'a' (and any char) returns it verbatim. Absent params
/// read as an empty string with status 0.
inline std::string sc_get_string_param(const char* name, char format_char = 'a', int* status = 0) {
    (void)format_char;
    int f = 0; const char* v = xsc_get_string_param(name, "", &f);
    std::string out = v ? v : ""; if (status) *status = f; return out;
}
inline int sc_get_param(const char* name, int& v) {
    int f = 0; v = xsc_get_int_param(name, 0, &f); return f;
}
inline int sc_get_param(const char* name, double& v) {
    int f = 0; v = xsc_get_real_param(name, 0.0, &f); return f;
}
inline int sc_get_param(const char* name, std::string& v, char format_char = 'a') {
    (void)format_char; int f = 0; const char* s = xsc_get_string_param(name, "", &f);
    v = s ? s : ""; return f;
}

// C ABI the boundary uses to feed the parameter table (implemented in
// sc_shim.cc). The SystemVerilog side calls xsc_set_param_count(0) then
// xsc_set_param_at(i, name, value) — or xsc_set_param_at(-1, ...) to
// replace-or-append — before the SC module reads its parameters.
extern "C" void xsc_set_param_count(int n);
extern "C" void xsc_set_param_at(int idx, const char* name, const char* value);
extern "C" int xsc_param_count(void);
extern "C" const char* xsc_param_name(int idx);
extern "C" const char* xsc_param_value(int idx);

#endif  // XEZIM_SC_API_H
