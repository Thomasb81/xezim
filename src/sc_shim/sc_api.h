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
void reg_signal4(const char* name, int width,
                 std::function<void(const std::uint64_t*, const std::uint64_t*, int)>&& writer,
                 std::function<void(std::uint64_t*, std::uint64_t*, int)>&& reader);

inline void reg_signal(const char* name, int width,
                       std::function<void(const std::uint64_t*, int)>&& writer,
                       std::function<void(std::uint64_t*, int)>&& reader) {
    reg_signal4(
        name, width,
        [w = std::move(writer)](const std::uint64_t* val, const std::uint64_t*, int n) {
            w(val, n);
        },
        [r = std::move(reader)](std::uint64_t* val, std::uint64_t* xz, int n) {
            for (int j = 0; j < n; ++j) xz[j] = 0;
            r(val, n);
        });
}
}  // namespace detail

/// Bind an sc_signal<bool> under `name`. Width 1 (2-state: SV X/Z -> false).
inline void xsc_bind(const char* name, sc_core::sc_signal<bool>& sig) {
    detail::reg_signal4(
        name, 1,
        [&sig](const std::uint64_t* w, const std::uint64_t* x, int) {
            const std::uint64_t known = w[0] & ~(x ? x[0] : 0ULL);
            sig.write((known & 1ULL) != 0);
        },
        [&sig](std::uint64_t* w, std::uint64_t* x, int) {
            w[0] = sig.read() ? 1 : 0;
            x[0] = 0;
        });
}

/// Bind an sc_signal of any small integer type (unsigned, int, std::uint64_t,
/// ...) under `name`. Width 64 (2-state: SV X/Z bits coerce to 0).
template <typename T,
          typename = std::enable_if_t<std::is_integral_v<T> && !std::is_same_v<T, bool>>>
inline void xsc_bind(const char* name, sc_core::sc_signal<T>& sig) {
    detail::reg_signal4(
        name, 64,
        [&sig](const std::uint64_t* w, const std::uint64_t* x, int) {
            const std::uint64_t known = w[0] & ~(x ? x[0] : 0ULL);
            sig.write(static_cast<T>(known));
        },
        [&sig](std::uint64_t* w, std::uint64_t* x, int) {
            w[0] = static_cast<std::uint64_t>(sig.read());
            x[0] = 0;
        });
}

/// Bind an sc_signal<sc_dt::sc_logic> under `name`. Width 1, full 4-state
/// (0/1/X/Z preserved in both directions).
inline void xsc_bind(const char* name, sc_core::sc_signal<sc_dt::sc_logic>& sig) {
    detail::reg_signal4(
        name, 1,
        [&sig](const std::uint64_t* w, const std::uint64_t* x, int) {
            const bool vb = (w[0] & 1ULL) != 0;
            const bool xb = x && ((x[0] & 1ULL) != 0);
            const sc_dt::sc_logic_value_t lv = xb
                ? (vb ? sc_dt::Log_Z : sc_dt::Log_X)
                : (vb ? sc_dt::Log_1 : sc_dt::Log_0);
            sig.write(sc_dt::sc_logic(lv));
        },
        [&sig](std::uint64_t* w, std::uint64_t* x, int) {
            const sc_dt::sc_logic v = sig.read();
            const sc_dt::sc_logic_value_t lv = v.value();
            w[0] = (lv == sc_dt::Log_1 || lv == sc_dt::Log_Z) ? 1ULL : 0ULL;
            x[0] = (lv == sc_dt::Log_X || lv == sc_dt::Log_Z) ? 1ULL : 0ULL;
        });
}

/// Bind an sc_signal<sc_dt::sc_lv<W>> under `name` (arbitrary width W >= 1).
/// Full 4-state: 0=(v=0,xz=0), 1=(v=1,xz=0), X=(v=0,xz=1), Z=(v=1,xz=1).
template <int W>
inline void xsc_bind(const char* name, sc_core::sc_signal<sc_dt::sc_lv<W>>& sig) {
    static_assert(W >= 1, "xsc_bind: width must be >= 1");
    constexpr int NW = (W + 63) / 64;
    detail::reg_signal4(
        name, W,
        [&sig](const std::uint64_t* w, const std::uint64_t* x, int) {
            sc_dt::sc_lv<W> v;
            for (int b = 0; b < W; ++b) {
                const bool vb = ((w[b / 64] >> (b % 64)) & 1ULL) != 0;
                const bool xb = x && (((x[b / 64] >> (b % 64)) & 1ULL) != 0);
                const sc_dt::sc_logic_value_t bit = xb
                    ? (vb ? sc_dt::Log_Z : sc_dt::Log_X)
                    : (vb ? sc_dt::Log_1 : sc_dt::Log_0);
                v.set_bit(b, bit);
            }
            sig.write(v);
        },
        [&sig](std::uint64_t* w, std::uint64_t* x, int) {
            for (int j = 0; j < NW; ++j) { w[j] = 0; x[j] = 0; }
            const sc_dt::sc_lv<W> v = sig.read();
            for (int b = 0; b < W; ++b) {
                const auto bit = v.get_bit(b);
                if (bit == sc_dt::Log_1 || bit == sc_dt::Log_Z)
                    w[b / 64] |= std::uint64_t(1) << (b % 64);
                if (bit == sc_dt::Log_X || bit == sc_dt::Log_Z)
                    x[b / 64] |= std::uint64_t(1) << (b % 64);
            }
        });
}

/// Bind an `sc_signal<sc_dt::sc_uint<W>>` under `name` (2-state: SV X/Z -> 0).
template <int W>
inline void xsc_bind(const char* name, sc_core::sc_signal<sc_dt::sc_uint<W>>& sig) {
    static_assert(W >= 1 && W <= 64, "xsc_bind: sc_uint width must be 1..64");
    detail::reg_signal4(
        name, W,
        [&sig](const std::uint64_t* w, const std::uint64_t* x, int) {
            const std::uint64_t known = w[0] & ~(x ? x[0] : 0ULL);
            const std::uint64_t mask = (W >= 64) ? ~std::uint64_t(0) : ((std::uint64_t(1) << W) - 1);
            sig.write(sc_dt::sc_uint<W>(known & mask));
        },
        [&sig](std::uint64_t* w, std::uint64_t* x, int) {
            w[0] = static_cast<std::uint64_t>(sig.read());
            x[0] = 0;
        });
}

/// Bind an `sc_signal<sc_dt::sc_bv<W>>` under `name` (2-state: SV X/Z -> 0).
template <int W>
inline void xsc_bind(const char* name, sc_core::sc_signal<sc_dt::sc_bv<W>>& sig) {
    static_assert(W >= 1, "xsc_bind: sc_bv width must be >= 1");
    constexpr int NW = (W + 63) / 64;
    detail::reg_signal4(
        name, W,
        [&sig](const std::uint64_t* w, const std::uint64_t* x, int) {
            sc_dt::sc_bv<W> v;
            for (int b = 0; b < W; ++b) {
                const bool vb = ((w[b / 64] >> (b % 64)) & 1ULL) != 0;
                const bool xb = x && (((x[b / 64] >> (b % 64)) & 1ULL) != 0);
                v[b] = (vb && !xb) ? sc_dt::SC_LOGIC_1 : sc_dt::SC_LOGIC_0;
            }
            sig.write(v);
        },
        [&sig](std::uint64_t* w, std::uint64_t* x, int) {
            for (int j = 0; j < NW; ++j) { w[j] = 0; x[j] = 0; }
            const sc_dt::sc_bv<W> v = sig.read();
            for (int b = 0; b < W; ++b)
                if (v[b] == sc_dt::SC_LOGIC_1) w[b / 64] |= std::uint64_t(1) << (b % 64);
        });
}

/// 4-state wrapper value returned by `XezSignal<W>::read()`: behaves as an
/// `sc_dt::sc_lv<W>` while implicitly converting to `uint64_t` / `bool` (with
/// X/Z coerced to 0) so existing integer testbenches compile unchanged.
template <int W>
struct XezLv : public sc_dt::sc_lv<W> {
    using sc_dt::sc_lv<W>::sc_lv;
    XezLv() : sc_dt::sc_lv<W>() {}
    XezLv(const sc_dt::sc_lv<W>& v) : sc_dt::sc_lv<W>(v) {}
    XezLv(const sc_dt::sc_bv<W>& v) : sc_dt::sc_lv<W>(v) {}
    operator std::uint64_t() const {
        std::uint64_t acc = 0;
        const int lim = W < 64 ? W : 64;
        for (int i = 0; i < lim; ++i) {
            if (this->get_bit(i) == sc_dt::Log_1) acc |= (std::uint64_t(1) << i);
        }
        return acc;
    }
};

/// 4-state SystemC port backing signal used by auto-derived SV wrappers
/// (`XezTop` and embedded `xez_<SvMod>.h` modules). Wraps `sc_signal<sc_lv<W>>`
/// so 0/1/X/Z flow end-to-end while preserving `write(int/bool/const char*)`,
/// `uint64_t q = sig.read()`, and direct binding to `sc_in/sc_out<sc_lv<W>>`.
template <int W>
struct XezSignal {
    sc_core::sc_signal<sc_dt::sc_lv<W>> sig;
    explicit XezSignal(const char* name) : sig(name) {}
    XezLv<W> read() const { return XezLv<W>(sig.read()); }
    void write(const sc_dt::sc_lv<W>& v) { sig.write(v); }
    void write(const sc_dt::sc_bv<W>& v) { sig.write(sc_dt::sc_lv<W>(v)); }
    void write(const char* s) { sig.write(sc_dt::sc_lv<W>(s)); }
    void write(bool v) { sig.write(sc_dt::sc_lv<W>(v ? 1 : 0)); }
    void write(int v) { sig.write(sc_dt::sc_lv<W>(v)); }
    void write(unsigned int v) { sig.write(sc_dt::sc_lv<W>(v)); }
    void write(long v) { sig.write(sc_dt::sc_lv<W>(v)); }
    void write(unsigned long v) { sig.write(sc_dt::sc_lv<W>(v)); }
    void write(long long v) { sig.write(sc_dt::sc_lv<W>(v)); }
    void write(unsigned long long v) { sig.write(sc_dt::sc_lv<W>(v)); }
    void write(const sc_dt::sc_logic& l) {
        sc_dt::sc_lv<W> v(0);
        v[0] = l;
        sig.write(v);
    }
    operator sc_core::sc_signal<sc_dt::sc_lv<W>>&() { return sig; }
    operator const sc_core::sc_signal<sc_dt::sc_lv<W>>&() const { return sig; }
    const sc_core::sc_event& default_event() const { return sig.default_event(); }
    const sc_core::sc_event& value_changed_event() const { return sig.value_changed_event(); }
};

template <int W>
inline void xsc_bind(const char* name, XezSignal<W>& s) {
    xsc_bind(name, s.sig);
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

// 1 when the SV kernel has staged an export the SystemC side has not yet
// consumed (see xsc_cosim_step's drain-to-quiescence loop). 0 otherwise.
extern "C" int xsc_sv_pending_exports(void);

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

// ── Multi-hop SV island registration (C++ SC_MODULE instantiating SV) ───────
extern "C" void xsc_reg_sv_island(const char* sc_inst, const char* sv_mod);

namespace xezim {
inline void xsc_register_sv_island(const char* sc_inst, const char* sv_mod) {
    xsc_reg_sv_island(sc_inst, sv_mod);
}
}  // namespace xezim

#endif  // XEZIM_SC_API_H
