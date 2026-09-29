//! VPI object kinds of implicit nets and array reads.
//!
//! Two elaboration bugs showed through VPI:
//!
//! * A continuous assignment READING an unpacked-array element
//!   (`assign w2 = m1[2];`) made the §6.10 implicit-net pass register a 1-bit
//!   placeholder net named after the array, so `vpi_iterate(vpiNet)` listed a
//!   phantom `top.m1` and `vpi_handle_by_name("top.m1")` found it.
//! * §23.2.2.3: a port with no net type and no data type (`input a`,
//!   `output [3:0] d`, a non-ANSI `input a;` never redeclared) is a net of the
//!   default net type, and so is an ANSI `input logic a`. Only ports spelled
//!   with an explicit `wire` were recorded as nets, so the rest reported
//!   `vpiReg`, in sub-modules and at the top alike. A non-ANSI port completed
//!   by `reg d;` stays a variable, as does an explicitly typed output and a
//!   non-ANSI `input logic e;`.
//!
//! Expected kinds are the reference simulator's.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn shared_lib(dir: &Path, stem: &str, src: &str) -> PathBuf {
    let c = dir.join(format!("{stem}.c"));
    let so = dir.join(format!("{stem}.so"));
    std::fs::write(&c, src).unwrap();
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let ok = Command::new("cc")
        .args(["-shared", "-fPIC", "-I"])
        .arg(&include)
        .arg(&c)
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {}", c.display());
    so
}

fn run(dir: &Path, lib: &Path, sv: &str) -> String {
    std::fs::write(dir.join("top.sv"), sv).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--dpi-lib")
        .arg(lib)
        .args(["--no-cache", "-s", "top", "top.sv"])
        .current_dir(dir)
        .output()
        .expect("failed to run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    text
}

const SV: &str = r#"
module s1 (a, y, d, e, f);
  input a;
  output y;
  output d;
  reg d;
  input logic e;
  output [1:0] f;
  assign y = ~a;
  assign f = {a, a};
  initial d = 0;
endmodule
module s2 (input a, input wire b, input logic [3:0] c, output y, output logic z,
           output var v);
  assign y = a & b;
  assign z = c[0];
  assign v = a;
endmodule
module top (input tin);
  logic [7:0] m1 [3:0];
  wire [7:0] w2;
  assign w2 = m1[2];
  reg r;
  wire y1, d1, y2, z2, v2;
  wire [1:0] f1;
  s1 u1 (.a(r), .y(y1), .d(d1), .e(r), .f(f1));
  s2 u2 (.a(r), .b(r), .c(4'h1), .y(y2), .z(z2), .v(v2));
  import "DPI-C" context function void probe();
  initial begin
    m1[2] = 8'h5a;
    r = 0;
    #1 probe();
    $finish;
  end
endmodule
"#;

const C: &str = r#"
#include <stdio.h>
#include "vpi_user.h"
static const char *kind(int t) {
    return t == vpiNet ? "net" : t == vpiReg ? "reg" : "other";
}
static void list_nets(const char *scope) {
    vpiHandle it = vpi_iterate(vpiNet, vpi_handle_by_name((PLI_BYTE8 *)scope, NULL));
    vpiHandle h;
    while (it && (h = vpi_scan(it))) vpi_printf("NET|%s\n", vpi_get_str(vpiFullName, h));
}
void probe(void) {
    list_nets("top");
    vpiHandle m = vpi_handle_by_name("top.m1", NULL);
    vpi_printf("M1|%s\n", m && vpi_get(vpiType, m) == vpiNet ? "net" : "not-net");
    const char *names[] = {"top.tin", "top.u1.a", "top.u1.y", "top.u1.d", "top.u1.e",
                           "top.u1.f", "top.u2.a", "top.u2.b", "top.u2.c", "top.u2.y",
                           "top.u2.z", "top.u2.v", 0};
    for (int i = 0; names[i]; i++) {
        vpiHandle h = vpi_handle_by_name((PLI_BYTE8 *)names[i], NULL);
        vpi_printf("KIND|%s=%s\n", names[i], h ? kind(vpi_get(vpiType, h)) : "null");
    }
    vpiHandle w2 = vpi_handle_by_name("top.w2", NULL);
    s_vpi_value v;
    v.format = vpiHexStrVal;
    vpi_get_value(w2, &v);
    vpi_printf("W2|%s\n", v.value.str);
}
"#;

#[test]
fn array_reads_add_no_net_and_implicit_ports_are_nets() {
    let d = scratch("vpi_net_kinds");
    let lib = shared_lib(&d, "probe", C);
    let text = run(&d, &lib, SV);
    let _ = std::fs::remove_dir_all(&d);
    // The object model iterates in declaration order; the set is what counts.
    let mut nets: Vec<&str> = text.lines().filter(|l| l.starts_with("NET|")).collect();
    nets.sort_unstable();
    assert_eq!(
        nets,
        [
            "NET|top.d1",
            "NET|top.f1",
            "NET|top.tin",
            "NET|top.v2",
            "NET|top.w2",
            "NET|top.y1",
            "NET|top.y2",
            "NET|top.z2"
        ],
        "{text}"
    );
    for line in [
        "M1|not-net",
        "KIND|top.tin=net",
        "KIND|top.u1.a=net",
        "KIND|top.u1.y=net",
        "KIND|top.u1.d=reg",
        "KIND|top.u1.e=reg",
        "KIND|top.u1.f=net",
        "KIND|top.u2.a=net",
        "KIND|top.u2.b=net",
        "KIND|top.u2.c=net",
        "KIND|top.u2.y=net",
        "KIND|top.u2.z=reg",
        "KIND|top.u2.v=reg",
        "W2|5a",
    ] {
        assert!(text.contains(line), "missing `{line}`:\n{text}");
    }
}
