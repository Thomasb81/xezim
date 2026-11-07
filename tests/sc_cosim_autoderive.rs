//! SystemC co-simulation: auto-derived wrapper (SC top).
//!
//! A pure SystemC testbench instantiates the generated `XezTop` wrapper whose
//! public `sc_signal` members mirror the SystemVerilog top module's ports, and
//! drives it with plain `sc_start()` — no `xsc_bind`, no `--sc-map`, no
//! `$sc_*` tasks.
//!
//! Gated: the test needs a working SystemC installation (`SYSTEMC_HOME`,
//! headers + `libsystemc`) and `g++`. When those aren't present it skips so
//! the plain-SV suite still passes on machines without SystemC.

use std::path::Path;
use std::process::Command;

fn sc_available() -> Option<String> {
    let sc_home = std::env::var("SYSTEMC_HOME").ok()?;
    if !Path::new(&format!("{sc_home}/include/systemc")).exists() {
        return None;
    }
    // The shim dir ships with the crate.
    let shim = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sc_shim");
    if !shim.join("sc_api.h").exists() {
        return None;
    }
    Some(shim.to_string_lossy().into_owned())
}

fn run_sc_top(sv: &str, sc: &str, top: &str) -> String {
    let exe = env!("CARGO_BIN_EXE_xezim");
    let dir = std::env::temp_dir();
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let svp = dir.join(format!("xez_scav_{pid}_{n}.sv"));
    let scp = dir.join(format!("xez_sctb_{pid}_{n}.cc"));
    std::fs::write(&svp, sv).unwrap();
    std::fs::write(&scp, sc).unwrap();
    let shim = sc_available().unwrap();
    let out = Command::new(exe)
        .args(["--simulate", "-s", top, "-I", &shim, "--sc-source"])
        .arg(&scp)
        .arg(&svp)
        .env("SYSTEMC_HOME", std::env::var("SYSTEMC_HOME").unwrap())
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_file(&svp);
    let _ = std::fs::remove_file(&scp);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Run the SV-master (`--sc-module`) direction: the SV top elaborates, then a
/// pure-SystemC module named `module` is instantiated at the boundary and a
/// plain-SV `module top` reads inputs / gets outputs. `extra` holds any extra
/// args (e.g. `-GNAME=VAL` parameter overrides).
fn run_sv_top(sv: &str, sc: &str, module: &str, top: &str, extra: &[String]) -> String {
    let exe = env!("CARGO_BIN_EXE_xezim");
    let dir = std::env::temp_dir();
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let svp = dir.join(format!("xez_svav_{pid}_{n}.sv"));
    let scp = dir.join(format!("xez_scmod_{pid}_{n}.cc"));
    std::fs::write(&svp, sv).unwrap();
    std::fs::write(&scp, sc).unwrap();
    let shim = sc_available().unwrap();
    let mut cmd = Command::new(exe);
    cmd.args(["--simulate", "-s", top, "-I", &shim, "--sc-module", module]);
    for a in extra {
        cmd.arg(a);
    }
    cmd.arg("--sc-source").arg(&scp).arg(&svp)
        .env("SYSTEMC_HOME", std::env::var("SYSTEMC_HOME").unwrap());
    let out = cmd.output().expect("run xezim");
    let _ = std::fs::remove_file(&svp);
    let _ = std::fs::remove_file(&scp);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn sc_top_auto_wrapper_combinational() {
    // Combinational DUT: dout = din + 1 across the SV↔SystemC boundary.
    let sv = r#"
module sv_incr(input logic [7:0] din, output logic [7:0] dout);
  assign dout = din + 8'd1;
endmodule
"#;
    let sc = r#"
#include <systemc>
#include <cstdio>
#include "xez_top.h"
int sc_main(int, char**) {
  XezTop dut("dut");
  dut.din.write(41);
  sc_start();
  uint64_t q = dut.dout.read();
  std::printf("TAG_%s dout=%llu\n", (q == 42) ? "PASS" : "FAIL",
              (unsigned long long)q);
  return 0;
}
"#;
    if sc_available().is_none() {
        eprintln!("SystemC/g++ unavailable; skipping SC co-sim test");
        return;
    }
    let out = run_sc_top(sv, sc, "sv_incr");
    assert!(
        out.contains("TAG_PASS dout=42"),
        "expected TAG_PASS dout=42, got:\n{out}"
    );
}

#[test]
fn sc_top_auto_wrapper_registered() {
    // Sequential SC-top DUT with an always_ff register: the testbench drives
    // one imported input/clock edge per sc_start() (reset, then assert en
    // with din=41, then a posedge). Because each sc_start() maps to a
    // drain-to-quiescence slice loop and the SV side parks between steps (SC
    // master still in sc_main), every later sc_start() must still reach the
    // SV kernel — not be swallowed by a premature "SV finished" — and its
    // NBA-committed register output must be flushed back before the final
    // read. This pins the multi-slice/registered case that a plain
    // single-slice interposer cannot deliver.
    let sv = r#"
module sv_adder(
  input  logic        clk,
  input  logic        en,
  input  logic        rst_n,
  input  logic [7:0]  din,
  output logic [7:0]  dout,
  output logic        valid
);
  logic [7:0] r;
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n)      r <= 8'd0;
    else if (en)     r <= din + 8'd1;
  end
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n)      valid <= 1'b0;
    else             valid <= en;
  end
  assign dout = r;
endmodule
"#;
    let sc = r#"
#include <systemc>
#include <cstdio>
#include "xez_top.h"
int sc_main(int, char**) {
  XezTop dut("dut");
  dut.rst_n.write(0); dut.en.write(0); dut.din.write(0); dut.clk.write(0);
  sc_start();                       // reset edge: r<=0, valid<=0
  dut.rst_n.write(1); dut.en.write(1); dut.din.write(41);
  sc_start();                       // prepare din/en
  dut.clk.write(1);
  sc_start();                       // posedge: r<=42, valid<=1
  uint64_t d = dut.dout.read();
  uint64_t v = dut.valid.read();
  std::printf("TAG_%s dout=%llu valid=%llu\n",
              (d == 42 && v == 1) ? "PASS" : "FAIL",
              (unsigned long long)d, (unsigned long long)v);
  return 0;
}
"#;
    if sc_available().is_none() {
        eprintln!("SystemC/g++ unavailable; skipping SC registered DUT test");
        return;
    }
    let out = run_sc_top(sv, sc, "sv_adder");
    assert!(
        out.contains("TAG_PASS dout=42 valid=1"),
        "expected the registered DUT to deliver dout=42 valid=1 across \
         multiple sc_start() steps, got:\n{out}"
    );
}

#[test]
fn sc_top_auto_wrapper_wide_crossing() {
    // >64-bit boundary crossings. The SC master writes a 100-bit input with
    // bit 80 set (requiring the high word), the SV combinational top adds 1,
    // and the 100-bit output must come back through the two-word import path
    // with bit 80 preserved. Before the wide gateway, `crossing_ok` declined
    // any port wider than 64 bits (warning + not auto-attached), so this is
    // the regression lock for widening the boundary to the u128 window.
    let sv = r#"
module sv_wide(
  input  logic [99:0] din,
  output logic [99:0] dout
);
  assign dout = din + 100'd1;
endmodule
"#;
    let sc = r#"
#include <systemc>
#include <cstdint>
#include <cstdio>
#include "xez_top.h"
int sc_main(int, char**) {
  XezTop dut("dut");
  sc_dt::sc_bv<100> bv(0);
  bv[80] = true;                 // set a bit strictly above word 0
  dut.din.write(sc_dt::sc_lv<100>(bv));
  sc_start();
  sc_dt::sc_lv<100> v = dut.dout.read();
  unsigned long long qlo = 0, qhi = 0;
  for (int i = 0; i < 64; i++) if (v[i].to_bool()) qlo |= (1ULL << i);
  for (int i = 64; i < 100; i++) if (v[i].to_bool()) qhi |= (1ULL << (i-64));
  std::printf("TAG_%s dout_lo=%llu dout_hi=%llx\n",
              (qlo == 1 && qhi == 0x10000ULL) ? "PASS" : "FAIL",
              (unsigned long long)qlo, (unsigned long long)qhi);
  return 0;
}
"#;
    if sc_available().is_none() {
        eprintln!("SystemC/g++ unavailable; skipping wide-crossing test");
        return;
    }
    let out = run_sc_top(sv, sc, "sv_wide");
    assert!(
        out.contains("TAG_PASS dout_lo=1 dout_hi=10000"),
        "expected the 100-bit crossing to preserve bit 80 (hi=0x10000) and \
         carry din+1 (lo=1) both ways, got:\n{out}"
    );
}

#[test]
fn sv_to_sc_parameter_passing() {
    // Pure SystemVerilog top whose `parameter`s are forwarded to a pure
    // SystemC module that reads them with sc_get_int_param (reference
    // `vlog_to_sc` pattern). Verify the default and a -G override both flow.
    let sv = r#"
module ptop #(parameter int counter_size = 4) (
  input logic clk, input logic rst_n, input logic [7:0] din,
  output logic [7:0] dout, output logic valid
);
endmodule
"#;
    let sc = r#"
#include <systemc>
#include <cstdint>
#include <cstdio>
#include "sc_api.h"
SC_MODULE(psc) {
  sc_core::sc_in<bool> clk;
  sc_core::sc_in<bool> rst_n;
  sc_core::sc_in<std::uint64_t> din;
  sc_core::sc_out<std::uint64_t> dout;
  sc_core::sc_out<bool> valid;
  SC_CTOR(psc) {
    int ok = 0;
    int cs = xsc_get_int_param("counter_size", -1, &ok);
    std::printf("SC_READ counter_size=%d found=%d\n", cs, ok);
  }
};
"#;
    if sc_available().is_none() {
        eprintln!("SystemC/g++ unavailable; skipping SC param test");
        return;
    }
    let out = run_sv_top(sv, sc, "psc", "ptop", &[]);
    assert!(
        out.contains("counter_size=4 found=1"),
        "expected default param 4, got:\n{out}"
    );
    // Override via the CLI knob (the SV-side #(.counter_size()) analog).
    let ok = vec!["-Gcounter_size=9".to_string()];
    let out2 = run_sv_top(sv, sc, "psc", "ptop", &ok);
    assert!(
        out2.contains("counter_size=9 found=1"),
        "expected -G override param 9, got:\n{out2}"
    );
}

#[test]
fn sv_to_sc_reference_param_family() {
    // Reference `param_passing/vlog_to_sc` source-compat: a pure-SV top whose
    // params include an int, a real, a string, and an X/Z bit-vector is read by
    // the SC module via the *standard* `sc_get_*_param` family (IEEE-1666
    // signatures), not the xezim `xsc_` fallback form. This locks in (1) the
    // reference-compatible `sc_get_real_param(name, int*)` / `sc_get_string_param
    // (name, char fmt, int*)` signatures and (2) X/Z bit-vector params being
    // forwarded as their width-padded binary string (`001100xz`), matching the
    // reference `results` file byte-for-byte.
    let sv = r#"
module preftop
  #(parameter int counter_size = 4,
    parameter int buffer_size = 16,
    parameter real real_param = 2.6,
    parameter int exp1_param = 6,
    parameter int exp2_param = 10,
    parameter string str_param = "Hellow World",
    parameter [7:0] range_param = 8'b001100xz)
  (input logic clk, input logic [7:0] din,
   output logic [7:0] dout);
endmodule
"#;
    let sc = r#"
#include <systemc>
#include <cstdint>
#include <cstdio>
#include <string>
#include "sc_api.h"
SC_MODULE(pref) {
  sc_core::sc_in<bool> clk;
  sc_core::sc_in<std::uint64_t> din;
  sc_core::sc_out<std::uint64_t> dout;
  SC_CTOR(pref) {
    int st = 0;
    int counter_size = sc_get_int_param("counter_size", &st);
    int buffer_size = 0;
    sc_get_param("buffer_size", buffer_size);
    double real_param = sc_get_real_param("real_param", &st);
    int exp = 0;
    sc_get_param("exp1_param", exp);
    std::string str_param, range_param;
    sc_get_param("str_param", str_param, 'a');
    range_param = sc_get_string_param("range_param", 'b', &st);
    std::printf("SC_PARAM counter=%d buffer=%d real=%.1f exp=%d str=%s range=%s\n",
                counter_size, buffer_size, real_param, exp, str_param.c_str(),
                range_param.c_str());
  }
};
"#;
    if sc_available().is_none() {
        eprintln!("SystemC/g++ unavailable; skipping SC param-family test");
        return;
    }
    let out = run_sv_top(sv, sc, "pref", "preftop", &[]);
    // Reference `results`: counter_size=4, buffer_size=16, real_param=2.6,
    // exp1_param=6, str_param=Hellow World, range_param=001100xz.
    assert!(
        out.contains("SC_PARAM counter=4 buffer=16 real=2.6 exp=6 str=Hellow World range=001100xz"),
        "reference param values not reproduced, got:\n{out}"
    );
}

/// Run the *netlist-leaf* Mode B direction: a plain-SV `module top` directly
/// instantiates a pure-SystemC module `module` (e.g. `inc_leaf u(.clock(clk),
/// .din(d), .dout(q))`) in its netlist. There is no SV `module inc_leaf`; the
/// placeholder is synthesized from `--sc-module` + `--sc-source` reflection,
/// and xezim auto-steps the SC kernel at each SV time advance (no `$sc_step`).
fn run_sv_leaf(module: &str, top: &str, sv: &str, sc: &str) -> String {
    let exe = env!("CARGO_BIN_EXE_xezim");
    let dir = std::env::temp_dir();
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let svp = dir.join(format!("xez_leaf_sv_{pid}_{n}.sv"));
    let scp = dir.join(format!("xez_leaf_sc_{pid}_{n}.cc"));
    std::fs::write(&svp, sv).unwrap();
    std::fs::write(&scp, sc).unwrap();
    let shim = sc_available().unwrap();
    let out = Command::new(exe)
        .args([
            "--simulate",
            "-s",
            top,
            "-I",
            &shim,
            "--sc-module",
            module,
            "--sc-source",
        ])
        .arg(&scp)
        .arg(&svp)
        .env("SYSTEMC_HOME", std::env::var("SYSTEMC_HOME").unwrap())
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_file(&svp);
    let _ = std::fs::remove_file(&scp);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn sv_netlist_instantiates_sc_leaf() {
    // Mode B: pure-SV top directly instantiates a pure-SystemC leaf. The SC
    // leaf registers din+1 on the posedge of its own clock and drives dout.
    // With automatic stepping, the SV testbench needs NO $sc_step. This is
    // the reference `vlog_sc` use-case shape (use-case 4): SV instantiates SC.
    let sv = r#"
module mbleaf;
  logic clk = 0;
  logic [7:0] din = 8'h2a;   // 42
  logic [7:0] dout;

  always #5 clk = ~clk;
  initial begin
    // Reference-parity (verified vs the reference simulator's sc_foreign_module
    // SC-top co-sim): a one-shot sample taken at the same instant as the
    // posedge reads the STALE value (0) — the edge-triggered SC output commits
    // through a later delta. A re-evaluated probe (`$monitor`) sees the
    // committed value (43) in the same slot; the held-value sample below sees
    // it too. Assert the committed value via `$monitor`, matching the reference
    // simulators' trace (see README §7: resume criteria answered: one-shot=0).
    @(posedge clk);
    $monitor("TAG_MB_COMMITTED dout=%0d", dout);
    #5  $display("TAG_MB dout=%0d", dout);   // committed value at t=10
    #2 $finish;
  end

  inc_leaf u(.clock(clk), .din(din), .dout(dout));
endmodule
"#;
    let sc = r#"
#include <systemc.h>
#include <cstdint>
#include "sc_api.h"
SC_MODULE(inc_leaf) {
public:
  sc_in<bool> clock;
  sc_in<sc_uint<8> > din;
  sc_out<sc_uint<8> > dout;
  sc_uint<8> reg_;
  SC_CTOR(inc_leaf) : clock("clock"), din("din"), dout("dout"), reg_(0) {
    SC_METHOD(sync);
    sensitive << clock.pos();
    dont_initialize();
  }
  void sync() {
    reg_ = (sc_uint<8>)(din.read() + 1);
    dout.write(reg_);
  }
};
"#;
    if sc_available().is_none() {
        eprintln!("SystemC/g++ unavailable; skipping SC leaf test");
        return;
    }
    let out = run_sv_leaf("inc_leaf", "mbleaf", sv, sc);
    assert!(
        out.contains("TAG_MB_COMMITTED dout=43"),
        "SC output must be visible in the same SV slot via a re-evaluated \
         probe, matching the reference; got:\n{out}"
    );
    assert!(
        out.contains("TAG_MB dout=43"),
        "expected the SC leaf to hold din+1 (43) on dout, got:\n{out}"
    );
}

#[test]
fn sv_netlist_instantiates_sc_leaf_twice() {
    // Mode B multi-instance (roadmap §5): the SAME pure-SystemC module
    // instantiated at two netlist sites must get two INDEPENDENT SC objects
    // with separate registers — not collapse onto one shared module. Before
    // the per-instance model fix this read q0=2 q1=2 (both sites shared one
    // `mod`, the last bind won); the correct read is q0=43 q1=2.
    let sv = r#"
module mb2;
  logic clk = 0;
  logic [7:0] din0 = 8'h2a;   // warm input for u0
  logic [7:0] din1 = 8'h01;   // distinct input for u1
  logic [7:0] q0, q1;
  always #5 clk = ~clk;
  initial begin
    // Independent per-instance SC objects: u0 holds din0+1 (43), u1 holds
    // din1+1 (2). Use a one-shot #6 sample (not the t=5 posedge slot the
    // deferred §7 issue touches) so this test validates independence, not the
    // deferred same-slot timing question.
    #6  $display("TAG2 q0=%0d q1=%0d", q0, q1);
    #2 $finish;
  end
  inc_leaf u0(.clock(clk), .din(din0), .dout(q0));
  inc_leaf u1(.clock(clk), .din(din1), .dout(q1));
endmodule
"#;
    let sc = r#"
#include <systemc.h>
#include <cstdint>
#include "sc_api.h"
SC_MODULE(inc_leaf) {
public:
  sc_in<bool> clock;
  sc_in<sc_uint<8> > din;
  sc_out<sc_uint<8> > dout;
  sc_uint<8> reg_;
  SC_CTOR(inc_leaf) : clock("clock"), din("din"), dout("dout"), reg_(0) {
    SC_METHOD(sync);
    sensitive << clock.pos();
    dont_initialize();
  }
  void sync() {
    reg_ = (sc_uint<8>)(din.read() + 1);
    dout.write(reg_);
  }
};
"#;
    if sc_available().is_none() {
        eprintln!("SystemC/g++ unavailable; skipping multi-instance leaf test");
        return;
    }
    let out = run_sv_leaf("inc_leaf", "mb2", sv, sc);
    assert!(
        out.contains("TAG2 q0=43 q1=2"),
        "two SC-leaf instances must keep independent state (u0=43, u1=2), \
         not collapse onto one shared module; got:\n{out}"
    );
}

#[test]
fn sv_leaf_sc_bv_lv_ports() {
    // Cousin of the leaf-model typing fix: the backing signals for reflected
    // `sc_in<sc_bv<8>>` / `sc_out<sc_lv<8>>` ports must be emitted as
    // `sc_dt::sc_bv<8>` / `sc_dt::sc_lv<8>` (namespace-qualified). The generated
    // model TU only `#include <systemc>` — it does not inherit a `using
    // namespace sc_dt`, so an unqualified `sc_bv<8>` failed to compile. This
    // locks in that the leaf passes an 8-bit value through a sc_bv/sc_lv
    // crossing.
    let sv = r#"
module mblv;
  logic clk = 0;
  logic [7:0] din = 8'h2a;
  logic [7:0] dout;
  always #5 clk = ~clk;
  initial begin
    // Reference-parity (README §7 resume criteria answered): a one-shot sample
    // at the posedge reads the STALE value in the reference; a re-evaluated
    // `$monitor` reads the committed 43 in the same slot.
    @(posedge clk);
    $monitor("TAG_LV_COMMITTED dout=%0d", dout);
    #2 $finish;
  end
  bvlv u(.clock(clk), .din(din), .dout(dout));
endmodule
"#;
    let sc = r#"
#include <systemc>
#include <cstdint>
#include "sc_api.h"
SC_MODULE(bvlv) {
  sc_core::sc_in<bool> clock;
  sc_core::sc_in<sc_dt::sc_bv<8>> din;
  sc_core::sc_out<sc_dt::sc_lv<8>> dout;
  SC_CTOR(bvlv) : clock("clock"), din("din"), dout("dout") {
    SC_METHOD(sync);
    sensitive << clock.pos();
    dont_initialize();
  }
  void sync() { dout.write(din.read()); }
};
"#;
    if sc_available().is_none() {
        eprintln!("SystemC/g++ unavailable; skipping sc_bv/sc_lv leaf test");
        return;
    }
    let out = run_sv_leaf("bvlv", "mblv", sv, sc);
    assert!(
        out.contains("TAG_LV_COMMITTED dout=42"),
        "sc_bv/sc_lv leaf must compile (sc_dt:: namespaced backing) and pass \
         din through unchanged (42) in the same slot;\n{out}"
    );
}