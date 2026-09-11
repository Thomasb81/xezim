// xezim SystemC co-simulation — user-facing API.
//
// Include this header from the C++ translation unit that builds the SystemC
// side of a co-simulation model and bind every SystemC signal that crosses
// the boundary with xsc_bind(). Bound signals form the co-simulation
// REGISTRY; the SystemVerilog side references them by the same name
// ($sc_export_sig / $sc_import_sig for a SystemVerilog top, --sc-map for a
// SystemC top).
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

#endif  // XEZIM_SC_API_H
