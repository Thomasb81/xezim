//! Auto-derived SV↔SC wrapper generation.
//!
//! The reference simulators (and Verilator) expose a systemVerilog top module
//! to a pure systemC testbench as an ordinary `sc_module` whose ports mirror
//! the SV top's ports, and let a pure systemVerilog testbench reference a
//! systemC module by its ports. xezim automates the same boundary instead of
//! asking the user the hand-write a bespoke boundary (`xsc_bind`/map calls):
//!
//!   SC top (`sc_main`):  xezim emits `xez_top.h` — an SC_MODULE (`XezTop`)
//!     whose public `sc_signal` members are the SV top's ports (name, width and
//!     direction taken from the elaborated SV netlist). The user's pure systemC
//!     `sc_main` instantiates `XezTop` and drives/reads those members with
//!     ordinary `sc_signal::write/read`, then calls `sc_start()` (interposed to
//!     the co-simulation slice loop by the generated header). No `xsc_bind`.
//!
//!   SV top (`--sc-module <Name>`):  xezim emits a generated TU that instantiates
//!     the user's systemC module `<Name>` and connects each crossing port to a
//!     backing `sc_signal` bound into the registry under the SV port's name. The
//!     SV top's own ports auto-attach by name (existing auto-attach), so both
//!     sides are reached with zero hand-written binding.
//!
//! The SV top's ports are only known after SV elaboration, which runs in the
//! worker thread. The generated text is therefore produced there and carried to
//! the build site (SC-top thread) or used directly (SV top) via the bridge.

use std::sync::Arc;

use super::link::ScLink;
use super::options::ScOptions;
use super::options::ScTop;
use super::options::{sc_module_ports, ScPort};

/// One systemVerilog top-module port, as needed to generate a SystemC
/// counterpart. Width and direction come straight from the elaborated netlist;
/// the SC type is derived from the width (`bool` at 1 bit, `uint64_t` up to 64,
/// `sc_lv<W>` beyond).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopPort {
    pub name: String,
    /// 1 = input, 2 = output, 3 = inout, 4 = ref (ref handled as inout).
    pub dir: ScPortDir,
    pub width: u32,
}

/// systemVerilog port direction, mirrored onto the SystemC crossing signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScPortDir {
    In,
    Out,
    Inout,
}

impl TopPort {
    /// The SystemC cross signal `sc_signal<T>` channel type for this port.
    pub fn sc_signal_type(&self) -> String {
        match self.width {
            0 => "sc_dt::sc_lv<1>".to_string(),
            1 => "bool".to_string(),
            2..=64 => "std::uint64_t".to_string(),
            w => format!("sc_dt::sc_lv<{w}>"),
        }
    }

    /// A C++ identifier sane enough to reference in generated code (SV names
    /// can contain characters illegal in C++ identifiers).
    pub fn ident(&self) -> String {
        sanitize_ident(&self.name)
    }

    fn sc_member(&self) -> String {
        format!(
            "sc_core::sc_signal<{}> {};",
            self.sc_signal_type(),
            self.ident()
        )
    }

    fn sc_bind(&self) -> String {
        format!(
            "xezim::xsc_bind(\"{}\", {});",
            self.name.replace('\\', "\\\\").replace('"', "\\\""),
            self.ident()
        )
    }
}

/// Generated wrapper artifacts carried from the SV worker (where the top
/// module's ports are known after elaboration) to the build site.
#[derive(Clone, Debug, Default)]
pub struct WrapperBundle {
    /// SC-master direction: `xez_top.h` header text (user `sc_main` includes it).
    pub sc_top_header: String,
    /// SC-master direction: `xez_top.cc` impl text (compiled with the model).
    pub sc_top_impl: String,
    /// SV-master direction: generated model TU(s) text (an `xsc_model_build`
    /// that instantiates the user systemC module and wires its ports).
    pub sv_top_sources: Vec<String>,
}

/// The active wrapper (top ports and generation mode) the SV worker settles.
static TOP_PORTS: std::sync::OnceLock<std::sync::Mutex<Vec<TopPort>>> = std::sync::OnceLock::new();

/// Record the elaborated SV top's ports so the build site can generate a
/// matching wrapper. Called by the simulator after elaboration.
pub fn set_top_ports(ports: Vec<TopPort>) {
    *TOP_PORTS
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .unwrap() = ports;
}

/// The SV top's ports, if recorded. The build always has access to them.
pub fn top_ports() -> Vec<TopPort> {
    TOP_PORTS
        .get()
        .map(|m| m.lock().unwrap().clone())
        .unwrap_or_default()
}

/// Auto-derive the SV<->SC wrapper from the recorded top ports and CLI intent.
///
/// * SC master: render `XezTop` (`xez_top.h`/`xez_top.cc`) and publish it on
///   the link so the SystemC host thread compiles it with the user model.
/// * SV master (`--sc-module <Name>`): render a generated `xsc_model_build`
///   harness instantiating the user systemC module; returned so the SV worker
///   can compile it into the model.
///
/// Always returns the bundle (used directly by the SV master; for the SC
/// master it is published for the other thread and returned for completeness).
pub fn generate_wrapper(opts: &ScOptions, link: Option<&Arc<ScLink>>) -> WrapperBundle {
    let ports = top_ports();
    let mut bundle = WrapperBundle::default();
    match opts.master {
        ScTop::Sc => {
            // systemC testbench instantiates the SV top.
            let (h, i) = render_sc_top_wrapper(&ports);
            bundle.sc_top_header = h;
            bundle.sc_top_impl = i;
        }
        ScTop::Sv => {
            // systemVerilog testbench instantiates a systemC module.
            if let Some(m) = opts.sc_module.as_deref() {
                // Mode B / leaf: bind the SC module's OWN reflected ports
                // (`sc_in`/`sc_out`/`sc_inout`), each under its bare name, so
                // the netlist's `.port(actual)` and the SV bridge can map the
                // instance's port signals to the registry by port name. Falls
                // back to the SV-top port names when reflection finds nothing
                // (an SC module with no `SC_MODULE(<name>) {` block in the
                // sources).
                let reflected = sc_module_ports(m, &opts.sources);
                let src = if reflected.is_empty() {
                    render_sv_top_model(m, &ports, &opts.sources)
                } else {
                    render_sv_leaf_model(m, &reflected, &opts.sources)
                };
                bundle.sv_top_sources.push(src);
            }
        }
    }
    if let Some(l) = link {
        l.publish_wrapper(bundle.clone());
    }
    bundle
}

/// Map a systemVerilog signal name to a valid C++ identifier (illegal chars ->
/// `_`, leading digit -> `_`).
pub fn sanitize_ident(name: &str) -> String {
    let mut s = String::with_capacity(name.len());
    let mut first = true;
    for ch in name.chars() {
        let ok_first = ch == '_' || ch.is_ascii_alphabetic();
        let ok_rest = ok_first || ch.is_ascii_digit();
        if (first && ok_first) || (!first && ok_rest) {
            s.push(ch);
        } else {
            s.push('_');
        }
        first = false;
    }
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    s
}

/// Every crossing must be representable as a 2-state word stream on the SV
/// side; beyond 64 bits is currently unsupported, matching the existing bridge.
fn crossing_ok(port: &TopPort) -> bool {
    port.width <= 64
}

/// Generated artifact for the SystemC master direction (SC top)
/// — an SC_MODULE named `XezTop` exposing the SV top's ports as public
/// `sc_signal` members bound into the co-simulation registry.
///
/// Returns (header_text, impl_text). The header is placed on the C++ include
/// path so the user's pure `sc_main` can `#include "xez_top.h"` and
/// instantiate `XezTop`. The impl defines the ctor (registers every crossing)
/// plus a `sc_start` interposition and a `xezim_sc_start()` helper driving the
/// slice loop.
fn render_sc_top_wrapper(ports: &[TopPort]) -> (String, String) {
    let mut members = String::new();
    let mut binds = String::new();
    for p in ports {
        if !crossing_ok(p) {
            continue;
        }
        members.push_str("  ");
        members.push_str(&p.sc_member());
        members.push('\n');
        binds.push_str("    ");
        binds.push_str(&p.sc_bind());
        binds.push('\n');
    }
    let header = format!(
        r#"// Auto-generated by xezim — do not edit.
// Pure-systemC wrapper exposing the systemVerilog top module's ports.
//
//   SC top (systemC instantiates systemVerilog):
//     #include "xez_top.h"
//     ...
//     XezTop dut("dut");
//     dut.<port>.write(v);       // drive an SV input
//     auto q = dut.<port>.read(); // read an SV output
//     sc_start();                 // interposed to the slice loop
//
// The boundary width is 64 bits (2-state); wider ports are not exposed.

#ifndef XEZ_TOP_H
#define XEZ_TOP_H

#include <cstdint>
#include <systemc>
#include "sc_api.h"

// Interpose the user's plain `sc_start(...)` onto the co-simulation driver so
// the systemC model does not need to know about the slice loop.
namespace xezim {{
/// Co-simulation driver: runs the SC-top slice loop (see sc_shim.cc).
extern "C" void xsc_cosim_run(void);
inline void sc_start_cosim() {{ xsc_cosim_run(); }}
}}  // namespace xezim
// Map the common sc_start() signatures onto the co-simulation driver.
#define sc_start(...) xezim::sc_start_cosim()

class XezTop : public sc_core::sc_module {{
public:
{members}  SC_HAS_PROCESS(XezTop);
  XezTop(sc_core::sc_module_name nm);
}};

#endif  // XEZ_TOP_H
"#,
        members = members,
    );
    let impl_text = format!(
        r#"// Auto-generated by xezim — do not edit.
// XezTop ctor: bind every crossing signal into the co-simulation registry
// under the SV top port's name. The SV side auto-attaches by name.
#include "xez_top.h"

XezTop::XezTop(sc_core::sc_module_name nm) : sc_module(nm) {{
{binds}}}
"#,
        binds = binds,
    );
    (header, impl_text)
}

/// Generated artifact for the systemVerilog master direction (SV top)
/// — a TU that instantiates the user's systemC module `<module>` and connects
/// each crossing port to a backing `sc_signal` bound into the registry under
/// the SV top port's name. This is what the reference simulators do under the
/// hood when the SV side references a systemC module: xezim only needs the
/// module NAME (`--sc-module`); port names/widths/directions come from the SV
/// top netlist.
pub fn render_sv_top_model(module: &str, ports: &[TopPort], user_sources: &[String]) -> String {
    let mut connects = String::new();
    let mut binds = String::new();
    for p in ports {
        if !crossing_ok(p) {
            continue;
        }
        let id = p.ident();
        let ty = p.sc_signal_type();
        let esc_name = p.name.replace('\\', "\\\\").replace('"', "\\\"");
        connects.push_str(&format!(
            "    sc_core::sc_signal<{}> _{}(\"{}\");  mod.{}(_{});\n",
            ty, id, esc_name, id, id
        ));
        binds.push_str(&format!(
            "    xezim::xsc_bind(\"{}\", _{}) ;\n",
            esc_name, id
        ));
    }
    let mut incs = String::new();
    for s in user_sources {
        // Source-path-include gives this TU the SC_MODULE definition so it
        // can instantiate the user module. Backslashes would be swallowed by
        // the C preprocessor as continuation lines, so map them to '/'.
        let esc = s.replace('\\', "/");
        incs.push_str(&format!("#include \"{esc}\"\n"));
    }
    format!(
        r#"// Auto-generated by xezim — do not edit.
// systemVerilog-master model harness: instantiate the user systemC module
// `{module}` and wire every SV-top port (by name) through the registry.
#include <cstdint>
#include <systemc>
#include "sc_api.h"
{incs}

extern "C" void xsc_model_build() {{
    static {module} mod("mod");
{connects}
{binds}}}"#,
        module = sanitize_ident(module),
        incs = incs,
        connects = connects,
        binds = binds,
    )
}

/// Leaf model for Mode B (a pure-SystemC module instantiated inside the SV
/// netlist). Instantiates the user SC module `<module>` and binds each of its
/// OWN reflected ports (`sc_in`/`sc_out`/`sc_inout`) to a backing
/// `sc_signal` registered in the registry under the bare port name. The SV
/// side then maps the netlist's instance port signals (`u.<port>`) to those
/// registry names; the output-port continuous assignments already synthesized
/// by the elaborator carry SC-driven values out to the parent actials.
///
/// `bool` for 1-bit ports, `uint64_t` up to 64 (the crossing limit), `sc_lv`
/// beyond.
pub fn render_sv_leaf_model(module: &str, ports: &[ScPort], user_sources: &[String]) -> String {
    // The backing `sc_signal<T>` must be the SAME template type as the SC
    // port (`sc_in<sc_uint<8>>` binds `sc_signal<sc_uint<8>>`, `sc_in<bool>`
    // binds `sc_signal<bool>`); using a width-only C type would fail the C++
    // port/signal type check at model-compile time. Prefer the reflected
    // `sc_type`; fall back to a width-based type only when reflection gave no
    // type (defensive).
    let sig_ty = |p: &ScPort| -> String {
        let t = p.sc_type.trim();
        if !t.is_empty() {
            // Qualify SystemC datatype templates (`sc_uint`, `sc_bv`, `sc_lv`,
            // `sc_reg`, `sc_int`, …) into `sc_dt::` — the generated model TU
            // only `#include <systemc>` and does not inherit a `using namespace
            // sc_dt`, so an unqualified `sc_bv<8>` would not compile (the
            // reference reflection strips `sc_dt::`, which is why we must add
            // it back here). `bool` / `int` / `std::uint64_t` are left as-is.
            if t.starts_with("sc_") && !t.starts_with("sc_core::")
                && !t.starts_with("std::")
            {
                return format!("sc_dt::{t}");
            }
            return t.to_string();
        }
        match p.width {
            0..=1 => "bool".to_string(),
            2..=64 => "std::uint64_t".to_string(),
            w => format!("sc_dt::sc_lv<{w}>"),
        }
    };
    let mut connects = String::new();
    let mut binds = String::new();
    for p in ports {
        let id = sanitize_ident(&p.name);
        let ty = sig_ty(p);
        let esc = p.name.replace('\\', "\\\\").replace('"', "\\\"");
        connects.push_str(&format!(
            "    static sc_core::sc_signal<{}> _{}(\"{}\");  mod.{}(_{});\n",
            ty, id, esc, id, id
        ));
        binds.push_str(&format!(
            "    xezim::xsc_bind(\"{}\", _{}) ;\n",
            esc, id
        ));
    }
    let mut incs = String::new();
    for s in user_sources {
        let esc = s.replace('\\', "/");
        incs.push_str(&format!("#include \"{esc}\"\n"));
    }
    format!(
        r#"// Auto-generated by xezim — do not edit.
// Mode B leaf model: instantiate the pure-systemC module `{module}` and bind
// its own ports (by name) into the registry.
#include <cstdint>
#include <systemc>
#include "sc_api.h"
{incs}

extern "C" void xsc_model_build() {{
    static {module} mod("mod");
{connects}{binds}}}"#,
        module = sanitize_ident(module),
        incs = incs,
        connects = connects,
        binds = binds,
    )
}