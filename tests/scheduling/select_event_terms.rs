//! §9.4.2 event terms that select bits of a vector.
//!
//! `always @(v[3])` wakes only when that bit changes, `@(v[7:4])` only when
//! one of those bits does, and an explicit event list never becomes
//! "whatever the body reads". Port substitution hands the simulator selects
//! OF selects (`@(dq[1])` on `inout [3:0] dq` wired to `bus[7:4]` arrives as
//! `bus[7:4][1]`); those resolve to the net bit they read and compile as
//! edge blocks, like a local `@(loc[1])`.
//!
//! Expected values are the reference simulator's.
use std::path::PathBuf;
use std::process::Command;

fn run(name: &str, src: &str, env: &[(&str, &str)]) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("select_event_terms");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join(format!("{name}.sv"));
    std::fs::write(&sv, src).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    for (k, v) in env {
        cmd.env(k, v);
    }
    let output = cmd
        .args(["--simulate", "-s", "tb", "--no-cache", sv.to_str().unwrap()])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    text
}

fn t_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|l| l.find("T|").map(|i| &l[i..]))
        .collect()
}

/// Bit and part selects of local vectors: ascending and non-zero-based
/// ranges read through their declared labels, two bits of one vector, a
/// bit next to the whole vector, a parameter index, `+:`, an element of a
/// packed 2-D vector, a bit past 64, and an edge on a part-select (its LSB).
#[test]
fn local_select_terms_wake_on_their_bits() {
    let o = run(
        "local",
        r#"
`timescale 1ns/1ns
module tb;
  localparam int K = 2;
  logic [0:3] a;
  logic [8:1] b;
  logic [7:0] c;
  logic [3:0][1:0] pk;
  logic [99:0] w;
  int na = 0, nb = 0, nc = 0, nd = 0, ne = 0, nf = 0, ng = 0, nh = 0, ni = 0, nj = 0, nk = 0, nl = 0, nm = 0;
  always @(a[1]) na++;
  always @(b[1]) nb++;
  always @(c[1] or c[2]) nc++;
  always @(posedge c[1] or negedge c[2]) nd++;
  always @(c[1] or c) ne++;
  always @(c[6:5]) nf++;
  always @(c[K +: 2]) ng++;
  always @(c[K]) nh++;
  always @(pk[1]) ni++;
  always @(w[70]) nj++;
  always @(posedge c[7:6]) nk++;
  always @(a[1:2]) nl++;
  always @(c[1] or posedge c) nm++;
  initial begin
    #1 a = 0; b = 0; c = 0; pk = 0; w = 0;
    for (int i = 0; i < 16; i++) begin #1 a = i; b = i * 3; c = i * 5; pk = i * 7; w = {i[3:0], 70'b0} | i; end
    #1 $display("T| na=%0d nb=%0d nc=%0d nd=%0d ne=%0d nf=%0d ng=%0d nh=%0d ni=%0d nj=%0d nk=%0d nl=%0d nm=%0d", na, nb, nc, nd, ne, nf, ng, nh, ni, nj, nk, nl, nm);
    $finish;
  end
endmodule
"#,
        &[],
    );
    assert_eq!(
        t_lines(&o),
        ["T| na=4 nb=16 nc=16 nd=9 ne=16 nf=3 ng=16 nh=13 ni=16 nj=16 nk=1 nl=8 nm=16"],
        "{o}"
    );
}

/// An explicit list with a select never takes the body's read set as its
/// sensitivity: the blocks call tasks and read `te` and `s`, and wake only
/// on `a[1]` / `a[3:2]` (the read-set routing woke them on every `s` and
/// `te` change and ran `viol` 8 times).
#[test]
fn select_list_ignores_body_reads() {
    let o = run(
        "b2",
        r#"
`timescale 1ps/1ps
module tb;
  logic [3:0] a = 0;
  logic       s = 0;
  real        te = -100.0;
  int         nv = 0, nw = 0, nx = 0;
  task automatic viol(); nv++; endtask
  task automatic hit_w(); nw++; endtask
  task automatic hit_x(); nx++; endtask
  always @(s) te = $realtime;
  // Explicit lists: each block wakes on its own bits only, never on what
  // its body reads (te, s) or on the other bits of `a`.
  always @(a[1]) begin
    hit_w();
    if ($realtime - te < 1.0) viol();
  end
  always @(a[3:2]) begin
    hit_x();
    if (s) viol();
  end
  initial begin
    #5;
    repeat (4) #10 s = ~s;
    repeat (3) #10 a[0] = ~a[0];
    #10 a = 4'b0110;
    #10 s = ~s;
    #10 a = 4'b1000;
    #10 $display("T| viol=%0d nw=%0d nx=%0d", nv, nw, nx);
    $finish;
  end
endmodule
"#,
        &[],
    );
    assert_eq!(t_lines(&o), ["T| viol=1 nw=2 nx=2"], "{o}");
}

/// Bit events on ports bound to selects of a parent net: inout part-selects
/// (`[7:4]`, `+:` from a generate, `-:`), a concatenation, a non-zero-based
/// net, two levels of ports, an output and an input; level, posedge and
/// negedge on one bit, and the whole port.
#[test]
fn port_bit_events_follow_the_connection() {
    let o = run(
        "ports",
        r#"
`timescale 1ns/1ns
module dev #(parameter string TAG="x") (inout [3:0] dq);
  int n1 = 0, np = 0, nn = 0, nw = 0;
  logic v1;
  always @(dq[1]) begin n1++; v1 = dq[1]; end
  always @(posedge dq[2]) np++;
  always @(negedge dq[3]) nn++;
  always @(dq) nw++;
  final $display("T| %s n1=%0d v1=%b np=%0d nn=%0d nw=%0d dq=%b", TAG, n1, v1, np, nn, nw, dq);
endmodule
module devo (output [3:0] oq, input [3:0] src);
  int n1 = 0;
  assign oq = src;
  always @(oq[1]) n1++;
  final $display("T| out n1=%0d", n1);
endmodule
module devi (input [3:0] iq);
  int n1 = 0, np = 0;
  always @(iq[1]) n1++;
  always @(posedge iq[3]) np++;
  final $display("T| in n1=%0d np=%0d", n1, np);
endmodule
module mid (inout [7:0] m);
  dev #("multi") u (.dq(m[7:4]));
endmodule
module tb;
  logic [15:0] v;
  wire [15:0] bus;  assign bus = v;
  wire [8:1] nz;    assign nz = v[15:8];
  wire [1:0] a2, b2; assign a2 = v[1:0]; assign b2 = v[3:2];
  wire [15:0] bus2; assign bus2 = v;
  wire [3:0] oo;
  dev  #("slice74")  u0 (.dq(bus[7:4]));
  for (genvar d = 0; d < 2; d++) begin : g
    dev #("plus") u (.dq(bus[d*4+8 +: 4]));
  end
  dev  #("minus")    um0 (.dq(bus[11 -: 4]));
  dev  #("concat")   u1 (.dq({a2, b2}));
  dev  #("nonzero")  u4 (.dq(nz[6:3]));
  dev  #("whole")    u5 (.dq(bus[3:0]));
  mid  um (.m(bus2[15:8]));
  devo uo (.oq(oo), .src(v[7:4]));
  devi ui (.iq(bus[11:8]));
  initial begin
    #1 v = 0;
    for (int i = 0; i < 16; i++) begin #1 v = 16'h1 << i; end
    for (int i = 0; i < 16; i++) begin #1 v = 16'h1111 * i; end
    #1 v = 16'hffff; #1 v = 16'h0; #1 v = 16'ha5a5; #1 v = 16'h5a5a; #1 v = 16'h1248;
    #1 $finish;
  end
endmodule
"#,
        &[],
    );
    assert_eq!(
        t_lines(&o),
        [
            "T| slice74 n1=13 v1=0 np=4 nn=4 nw=25 dq=0100",
            "T| plus n1=12 v1=1 np=4 nn=4 nw=25 dq=0010",
            "T| plus n1=13 v1=0 np=4 nn=4 nw=25 dq=0001",
            "T| minus n1=12 v1=1 np=4 nn=4 nw=25 dq=0010",
            "T| concat n1=6 v1=1 np=10 nn=7 nw=25 dq=0010",
            "T| nonzero n1=7 v1=0 np=10 nn=7 nw=25 dq=0100",
            "T| whole n1=13 v1=0 np=4 nn=3 nw=25 dq=1000",
            "T| multi n1=13 v1=0 np=4 nn=4 nw=25 dq=0001",
            "T| out n1=13",
            "T| in n1=12 np=3",
        ],
        "{o}"
    );
}

/// `@(dq[g])` on an inout port bit compiles as an edge block watching the
/// parent net (it ran as an interpreted process parked on the whole bus),
/// exactly like the local `@(loc[g])` beside it.
#[test]
fn port_bit_events_are_edge_blocks() {
    let o = run(
        "vw1",
        r#"
`timescale 1ps/1ps
module dev (inout [3:0] dq, input [3:0] din);
  int n = 0, m = 0;
  logic [3:0] loc;
  assign loc = din;
  genvar g;
  for (g = 0; g < 4; g++) begin : g_dq
    always @(dq[g]) n++;      // inout port bit
    always @(loc[g]) m++;     // local net bit
  end
endmodule
module tb;
  wire [7:0] bus;
  logic [7:0] v = 0;
  assign bus = v;
  for (genvar d = 0; d < 2; d++) begin : g_dev
    dev u (.dq(bus[d*4 +: 4]), .din(v[d*4 +: 4]));
  end
  initial begin repeat (3) #10 v = v + 8'h11; #5000; end
endmodule
"#,
        &[("XEZIM_DUMP_EDGE_SENS", "1")],
    );
    let edge: Vec<&str> = o.lines().filter(|l| l.starts_with("[EDGE-SENS]")).collect();
    assert_eq!(edge.len(), 16, "{o}");
    assert_eq!(
        edge.iter().filter(|l| l.ends_with("sens=[bus]")).count(),
        8,
        "{o}"
    );
}

/// §7.4.1 part-selects of ASCENDING vectors that are nets or live inside an
/// instance read the declared labels (`v[0:3]` is the four MSBs). They read
/// the bits mirrored, while bit-selects and top-level variables were right;
/// a level event on such a part-select and a port bound to one follow.
#[test]
fn ascending_part_selects_in_instances_and_nets() {
    let o = run(
        "asc",
        r#"
`timescale 1ns/1ns
module ch (input [7:0] src);
  logic [0:7] av;
  wire  [0:7] aw;
  logic [3:10] ao;
  assign aw = src;
  always @* begin av = src; ao = src; end
  final $display("T| ch av03=%b av47=%b aw03=%b aw47=%b av25=%b aw1=%b ao36=%b ao4=%b aw_up=%b aw_dn=%b",
                 av[0:3], av[4:7], aw[0:3], aw[4:7], av[2:5], aw[1], ao[3:6], ao[4], aw[2 +: 3], aw[6 -: 2]);
endmodule
module dev (inout [3:0] dq);
  int n1 = 0;
  always @(dq[1]) n1++;
  final $display("T| dev n1=%0d dq=%b", n1, dq);
endmodule
module tb;
  logic [7:0] v;
  wire [0:7] abus; assign abus = v;
  int na = 0;
  ch u (.src(v));
  dev d0 (.dq(abus[0:3]));
  always @(abus[2:3]) na++;
  initial begin
    #1 v = 0;
    for (int i = 0; i < 16; i++) begin #1 v = 8'h11 * i; end
    #1 v = 8'b0001_0010;
    #1 $display("T| tb abus03=%b abus47=%b na=%0d", abus[0:3], abus[4:7], na);
    $finish;
  end
endmodule
"#,
        &[],
    );
    assert_eq!(
        t_lines(&o),
        [
            "T| tb abus03=0001 abus47=0010 na=17",
            "T| ch av03=0001 av47=0010 aw03=0001 aw47=0010 av25=0100 aw1=0 ao36=0001 ao4=0 aw_up=010 aw_dn=01",
            "T| dev n1=9 dq=0001",
        ],
        "{o}"
    );
}
