//! SystemC co-simulation: 4-state (0/1/X/Z), wide (>128-bit) crossings, and
//! arbitrary-depth multi-hop mixed-language sandwiching (SV↔SC↔SV↔SC↔SV and
//! SC↔SV↔SC↔SV).
//!
//! Gated on `SYSTEMC_HOME` + `g++` like `sc_cosim_autoderive.rs`.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn sc_available() -> Option<String> {
    let sc_home = std::env::var("SYSTEMC_HOME").ok()?;
    if !Path::new(&format!("{sc_home}/include/systemc")).exists() {
        return None;
    }
    let shim = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sc_shim");
    if !shim.join("sc_api.h").exists() {
        return None;
    }
    Some(shim.to_string_lossy().into_owned())
}

fn run_xezim(sv: &str, sc: &str, top: Option<&str>, extra_args: &[&str]) -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir();
    let svp = dir.join(format!("xez_mh_sv_{pid}_{n}.sv"));
    let scp = dir.join(format!("xez_mh_sc_{pid}_{n}.cc"));
    std::fs::write(&svp, sv).unwrap();
    std::fs::write(&scp, sc).unwrap();
    let shim = sc_available().unwrap();
    let exe = env!("CARGO_BIN_EXE_xezim");
    let mut cmd = Command::new(exe);
    cmd.arg("--simulate");
    if let Some(t) = top {
        cmd.args(["-s", t]);
    }
    cmd.args(["-I", &shim]);
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.arg("--sc-source")
        .arg(&scp)
        .arg(&svp)
        .env("SYSTEMC_HOME", std::env::var("SYSTEMC_HOME").unwrap());
    let out = cmd.output().expect("run xezim");
    let _ = std::fs::remove_file(&svp);
    let _ = std::fs::remove_file(&scp);
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.stderr.is_empty() {
        combined.push_str("\n--- STDERR ---\n");
        combined.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    combined
}

#[test]
fn four_state_sc_top_roundtrip_and_wide() {
    let Some(_) = sc_available() else {
        return;
    };
    let sv = r#"
module sv_4s(
  input  logic         sel,
  input  logic [7:0]   din,
  output logic [7:0]   dout,
  input  logic [199:0] w_in,
  output logic [199:0] w_out
);
  assign dout  = (sel === 1'bz) ? 8'b11xxzz00 : (sel ? din : 8'bzzzzxxxx);
  assign w_out = w_in;
endmodule
"#;
    let sc = r#"
#include <systemc>
#include <cstdio>
#include <string>
#include "xez_top.h"

int sc_main(int, char**) {
  XezTop dut("dut");
  dut.sel.write(sc_dt::SC_LOGIC_Z);
  dut.din.write("10xz01zx");
  sc_dt::sc_lv<200> win(0);
  win[0]   = sc_dt::Log_X;
  win[1]   = sc_dt::Log_Z;
  win[190] = sc_dt::Log_X;
  win[191] = sc_dt::Log_Z;
  win[192] = sc_dt::Log_1;
  dut.w_in.write(win);
  sc_start();

  std::string d1 = dut.dout.read().to_string();
  sc_dt::sc_lv<200> wout = dut.w_out.read();
  bool wide_ok = (wout[0] == sc_dt::Log_X) && (wout[1] == sc_dt::Log_Z) &&
                 (wout[190] == sc_dt::Log_X) && (wout[191] == sc_dt::Log_Z) &&
                 (wout[192] == sc_dt::Log_1);

  dut.sel.write(true);
  sc_start();
  std::string d2 = dut.dout.read().to_string();

  dut.sel.write(false);
  sc_start();
  std::string d3 = dut.dout.read().to_string();

  std::printf("TAG_%s d1=%s d2=%s d3=%s wide=%d\n",
              (d1 == "11XXZZ00" && d2 == "10XZ01ZX" && d3 == "ZZZZXXXX" && wide_ok)
                  ? "PASS" : "FAIL",
              d1.c_str(), d2.c_str(), d3.c_str(), (int)wide_ok);
  return 0;
}
"#;
    let out = run_xezim(sv, sc, Some("sv_4s"), &[]);
    assert!(
        out.contains("TAG_PASS d1=11XXZZ00 d2=10XZ01ZX d3=ZZZZXXXX wide=1"),
        "4-state SC-top roundtrip failed:\n{out}"
    );
}

#[test]
fn four_state_sv_leaf_mode_b_sc_lv_and_sc_logic() {
    let Some(_) = sc_available() else {
        return;
    };
    let sv = r#"
module top_4s;
  logic       bit_in;
  logic       bit_out;
  logic [7:0] vec_in;
  logic [7:0] vec_out;

  leaf_4s u(
    .bit_in(bit_in),
    .bit_out(bit_out),
    .vec_in(vec_in),
    .vec_out(vec_out)
  );

  initial begin
    bit_in = 1'bx;
    vec_in = 8'b10xz01zx;
    #5;
    if (bit_out === 1'bx && vec_out === 8'b10xz01zx)
      $display("TAG_4S_1 PASS %b %b", bit_out, vec_out);
    else
      $display("TAG_4S_1 FAIL %b %b", bit_out, vec_out);

    bit_in = 1'bz;
    vec_in = 8'bzzzzxxxx;
    #5;
    if (bit_out === 1'bz && vec_out === 8'bzzzzxxxx)
      $display("TAG_4S_2 PASS %b %b", bit_out, vec_out);
    else
      $display("TAG_4S_2 FAIL %b %b", bit_out, vec_out);
    $finish;
  end
endmodule
"#;
    let sc = r#"
#include <systemc>
#include "sc_api.h"

SC_MODULE(leaf_4s) {
public:
  sc_core::sc_in<sc_dt::sc_logic>    bit_in{"bit_in"};
  sc_core::sc_out<sc_dt::sc_logic>   bit_out{"bit_out"};
  sc_core::sc_in<sc_dt::sc_lv<8>>    vec_in{"vec_in"};
  sc_core::sc_out<sc_dt::sc_lv<8>>   vec_out{"vec_out"};

  SC_CTOR(leaf_4s) {
    SC_METHOD(comb);
    sensitive << bit_in << vec_in;
  }
  void comb() {
    bit_out.write(bit_in.read());
    vec_out.write(vec_in.read());
  }
};
"#;
    let out = run_xezim(sv, sc, Some("top_4s"), &["--sc-module", "leaf_4s"]);
    assert!(
        out.contains("TAG_4S_1 PASS x 10xz01zx") && out.contains("TAG_4S_2 PASS z zzzzxxxx"),
        "Mode B 4-state sc_logic/sc_lv failed:\n{out}"
    );
}

#[test]
fn deep_sv_hierarchy_instantiates_sc_leaf() {
    let Some(_) = sc_available() else {
        return;
    };
    let sv = r#"
module sv_inner(input logic clk, input logic [7:0] d, output logic [7:0] q);
  inc_leaf u_sc(.clock(clk), .din(d), .dout(q));
endmodule

module sv_mid(input logic clk, input logic [7:0] d, output logic [7:0] q);
  sv_inner u_in(.clk(clk), .d(d), .q(q));
endmodule

module top_deep;
  logic clk = 0;
  logic [7:0] d0 = 8'd10, d1 = 8'd20;
  logic [7:0] q0, q1;
  always #5 clk = ~clk;

  sv_mid u0(.clk(clk), .d(d0), .q(q0));
  sv_mid u1(.clk(clk), .d(d1), .q(q1));

  initial begin
    #6;
    $display("TAG_DEEP q0=%0d q1=%0d", q0, q1);
    $finish;
  end
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
  SC_CTOR(inc_leaf) : clock("clock"), din("din"), dout("dout") {
    SC_METHOD(sync);
    sensitive << clock.pos();
    dont_initialize();
  }
  void sync() { dout.write((sc_uint<8>)(din.read() + 1)); }
};
"#;
    let out = run_xezim(sv, sc, Some("top_deep"), &[]);
    assert!(
        out.contains("TAG_DEEP q0=11 q1=21"),
        "Deep SV->SV->SC failed:\n{out}"
    );
}

#[test]
fn five_layer_sv_sc_sv_sc_sv_sandwich_multi_instance() {
    let Some(_) = sc_available() else {
        return;
    };
    // Depth 5 (4 language crossings):
    //   top_sw (SV0)
    //     -> sc_hop1 u0, u1 (SC1, two independent instances)
    //       -> sv_hop2 (SV2)
    //         -> sc_hop3 (SC3)
    //           -> sv_hop4 (SV4): adds 4 and preserves upper 4-state nibble
    let sv = r#"
module sv_hop4(input logic [7:0] din, output logic [7:0] dout);
  assign dout = {din[7:4], (din[3:0] + 4'd4)};
endmodule

module sv_hop2(input logic [7:0] din, output logic [7:0] dout);
  logic [7:0] mid;
  assign mid = {din[7:4], (din[3:0] + 4'd2)};
  sc_hop3 u3(.din(mid), .dout(dout));
endmodule

module top_sw;
  logic [7:0] a0, a1;
  logic [7:0] y0, y1;

  sc_hop1 u0(.din(a0), .dout(y0));
  sc_hop1 u1(.din(a1), .dout(y1));

  initial begin
    // Hop offsets: +1 (SC1) +2 (SV2) +3 (SC3) +4 (SV4) = +10 on low nibble;
    // upper nibble preserves 4-state `4'bx0z1` across all 4 crossings!
    a0 = 8'bx0z1_0001;  // low=1 -> 1+10=11 (4'hb)
    a1 = 8'b1z0x_0010;  // low=2 -> 2+10=12 (4'hc)
    #5;
    $display("TAG_SW5 y0=%b y1=%b", y0, y1);
    if (y0 === 8'bx0z1_1011 && y1 === 8'b1z0x_1100)
      $display("TAG_SW5_PASS");
    else
      $display("TAG_SW5_FAIL");
    $finish;
  end
endmodule
"#;
    let sc = r#"
#include <systemc>
#include "sc_api.h"
#include "xez_sv_hop2.h"
#include "xez_sv_hop4.h"

SC_MODULE(sc_hop3) {
public:
  sc_core::sc_in<sc_dt::sc_lv<8>>  din{"din"};
  sc_core::sc_out<sc_dt::sc_lv<8>> dout{"dout"};
  sv_hop4 u4{"u4"};

  SC_CTOR(sc_hop3) {
    SC_METHOD(fwd);
    sensitive << din;
    SC_METHOD(bwd);
    sensitive << u4.dout.value_changed_event();
  }
  void fwd() {
    sc_dt::sc_lv<8> v = din.read();
    unsigned lo = 0;
    for (int i = 0; i < 4; ++i)
      if (v[i] == sc_dt::Log_1) lo |= (1u << i);
    lo = (lo + 3) & 0xFu;
    for (int i = 0; i < 4; ++i)
      v[i] = ((lo >> i) & 1u) ? sc_dt::Log_1 : sc_dt::Log_0;
    u4.din.write(v);
  }
  void bwd() {
    dout.write(u4.dout.read());
  }
};

SC_MODULE(sc_hop1) {
public:
  sc_core::sc_in<sc_dt::sc_lv<8>>  din{"din"};
  sc_core::sc_out<sc_dt::sc_lv<8>> dout{"dout"};
  sv_hop2 u2{"u2"};

  SC_CTOR(sc_hop1) {
    SC_METHOD(fwd);
    sensitive << din;
    SC_METHOD(bwd);
    sensitive << u2.dout.value_changed_event();
  }
  void fwd() {
    sc_dt::sc_lv<8> v = din.read();
    unsigned lo = 0;
    for (int i = 0; i < 4; ++i)
      if (v[i] == sc_dt::Log_1) lo |= (1u << i);
    lo = (lo + 1) & 0xFu;
    for (int i = 0; i < 4; ++i)
      v[i] = ((lo >> i) & 1u) ? sc_dt::Log_1 : sc_dt::Log_0;
    u2.din.write(v);
  }
  void bwd() {
    dout.write(u2.dout.read());
  }
};
"#;
    let out = run_xezim(sv, sc, Some("top_sw"), &[]);
    assert!(
        out.contains("TAG_SW5_PASS"),
        "5-layer SV->SC->SV->SC->SV sandwich failed:\n{out}"
    );
}

#[test]
fn four_layer_sc_top_sv_sc_sv_sandwich() {
    let Some(_) = sc_available() else {
        return;
    };
    // Depth 4 (SC top):
    //   sc_main (SC0) -> XezTop sv_hop1 (SV1) -> sc_hop2 (SC2) -> sv_hop3 (SV3)
    let sv = r#"
module sv_hop3(input logic [7:0] din, output logic [7:0] dout);
  assign dout = {din[7:4], (din[3:0] + 4'd3)};
endmodule

module sv_hop1(input logic [7:0] din, output logic [7:0] dout);
  logic [7:0] mid;
  assign mid = {din[7:4], (din[3:0] + 4'd1)};
  sc_hop2 u2(.din(mid), .dout(dout));
endmodule
"#;
    let sc = r#"
#include <systemc>
#include <cstdio>
#include <string>
#include "xez_top.h"
#include "xez_sv_hop3.h"

SC_MODULE(sc_hop2) {
public:
  sc_core::sc_in<sc_dt::sc_lv<8>>  din{"din"};
  sc_core::sc_out<sc_dt::sc_lv<8>> dout{"dout"};
  sv_hop3 u3{"u3"};

  SC_CTOR(sc_hop2) {
    SC_METHOD(fwd);
    sensitive << din;
    SC_METHOD(bwd);
    sensitive << u3.dout.value_changed_event();
  }
  void fwd() {
    sc_dt::sc_lv<8> v = din.read();
    unsigned lo = 0;
    for (int i = 0; i < 4; ++i)
      if (v[i] == sc_dt::Log_1) lo |= (1u << i);
    lo = (lo + 2) & 0xFu;
    for (int i = 0; i < 4; ++i)
      v[i] = ((lo >> i) & 1u) ? sc_dt::Log_1 : sc_dt::Log_0;
    u3.din.write(v);
  }
  void bwd() {
    dout.write(u3.dout.read());
  }
};

int sc_main(int, char**) {
  XezTop dut("dut");
  // +1 (SV1) +2 (SC2) +3 (SV3) = +6 on low nibble (2 -> 8 = 1000),
  // upper nibble keeps `XZ10`!
  dut.din.write("xz100010");
  sc_start();
  std::string got = dut.dout.read().to_string();
  std::printf("TAG_SCTOP_%s got=%s\n",
              (got == "XZ101000") ? "PASS" : "FAIL", got.c_str());
  return 0;
}
"#;
    let out = run_xezim(sv, sc, Some("sv_hop1"), &[]);
    assert!(
        out.contains("TAG_SCTOP_PASS got=XZ101000"),
        "4-layer SC->SV->SC->SV sandwich failed:\n{out}"
    );
}

#[test]
fn clocked_multihop_sv_sc_sv_and_two_state_coercion() {
    let Some(_) = sc_available() else {
        return;
    };
    // Clocked 3-layer sandwich (`top_clk (SV0) -> sc_reg_hop (SC1) -> sv_reg_leaf (SV2)`)
    // plus 2-state (`bit`) vs 4-state (`logic`) SV receiver coercion of X/Z.
    let sv = r#"
module sv_reg_leaf(input logic clk, input logic [7:0] din, output logic [7:0] dout);
  logic [7:0] q_r;
  always_ff @(posedge clk) q_r <= din + 8'd2;
  assign dout = q_r;
endmodule

module top_clk;
  logic       clk = 0;
  logic [7:0] din = 8'd10;
  logic [7:0] dout_4s;
  logic [7:0] xz_in = 8'b10xz01zx;
  logic [7:0] xz_out_4s;
  bit   [7:0] xz_out_2s;

  always #5 clk = ~clk;

  sc_reg_hop u_sc(
    .clk(clk),
    .din(din),
    .dout(dout_4s),
    .xz_in(xz_in),
    .xz_out_4s(xz_out_4s),
    .xz_out_2s(xz_out_2s)
  );

  initial begin
    // Edge 1 at t=5: SC registers din+1 (11) into u_sv.din, SV registers it at next posedge (t=15).
    // Verify at t=16 that dout_4s == 13 (10 + 1 + 2), xz_out_4s === 8'b10xz01zx,
    // and xz_out_2s === 8'b10000100 (X/Z coerced to 0 on 2-state SV target).
    #16;
    $display("TAG_CLK_MH dout=%0d 4s=%b 2s=%b", dout_4s, xz_out_4s, xz_out_2s);
    if (dout_4s === 8'd13 && xz_out_4s === 8'b10xz01zx && xz_out_2s === 8'b10000100)
      $display("TAG_CLK_MH_PASS");
    else
      $display("TAG_CLK_MH_FAIL");
    $finish;
  end
endmodule
"#;
    let sc = r#"
#include <systemc>
#include <cstdint>
#include "sc_api.h"
#include "xez_sv_reg_leaf.h"

SC_MODULE(sc_reg_hop) {
public:
  sc_core::sc_in<bool>             clk{"clk"};
  sc_core::sc_in<sc_dt::sc_uint<8>> din{"din"};
  sc_core::sc_out<sc_dt::sc_lv<8>> dout{"dout"};
  sc_core::sc_in<sc_dt::sc_lv<8>>  xz_in{"xz_in"};
  sc_core::sc_out<sc_dt::sc_lv<8>> xz_out_4s{"xz_out_4s"};
  sc_core::sc_out<sc_dt::sc_lv<8>> xz_out_2s{"xz_out_2s"};

  sv_reg_leaf u_sv{"u_sv"};

  SC_CTOR(sc_reg_hop) {
    SC_METHOD(on_clk);
    sensitive << clk;
    SC_METHOD(on_dout);
    sensitive << u_sv.dout.value_changed_event();
    SC_METHOD(on_xz);
    sensitive << xz_in;
  }
  void on_clk() {
    u_sv.clk.write(clk.read());
    if (clk.read()) {
      u_sv.din.write((unsigned)(din.read() + 1));
    }
  }
  void on_dout() {
    dout.write(u_sv.dout.read());
  }
  void on_xz() {
    xz_out_4s.write(xz_in.read());
    xz_out_2s.write(xz_in.read());
  }
};
"#;
    let out = run_xezim(sv, sc, Some("top_clk"), &[]);
    assert!(
        out.contains("TAG_CLK_MH_PASS"),
        "Clocked multi-hop / 2-state coercion test failed:\n{out}"
    );
}

#[test]
fn arbitrary_width_8192_bit_four_state_crossing() {
    let Some(_) = sc_available() else {
        return;
    };
    let sv = r#"
module sv_8k(
  input  logic [8191:0] din,
  output logic [8191:0] dout
);
  assign dout = {~din[8191], din[8190:0]};
endmodule
"#;
    let sc = r#"
#include <systemc>
#include <cstdio>
#include "xez_top.h"

int sc_main(int, char**) {
  XezTop dut("dut");
  sc_dt::sc_lv<8192> v(0);
  v[0]    = sc_dt::Log_1;
  v[4095] = sc_dt::Log_X;
  v[5000] = sc_dt::Log_Z;
  v[8191] = sc_dt::Log_0;
  dut.din.write(v);
  sc_start();
  sc_dt::sc_lv<8192> q = dut.dout.read();
  bool ok = (q[0] == sc_dt::Log_1) &&
            (q[4095] == sc_dt::Log_X) &&
            (q[5000] == sc_dt::Log_Z) &&
            (q[8191] == sc_dt::Log_1);
  std::printf("TAG_8K_%s\n", ok ? "PASS" : "FAIL");
  return 0;
}
"#;
    let out = run_xezim(sv, sc, Some("sv_8k"), &[]);
    assert!(
        out.contains("TAG_8K_PASS"),
        "8192-bit 4-state crossing failed:\n{out}"
    );
}
