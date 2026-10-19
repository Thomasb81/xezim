//! SystemVerilog placeholder synthesis for a pure-SystemC leaf module (Mode B).
//!
//! When a SystemVerilog netlist instantiates a SystemC module directly
//! (`ringbuf u(.clock(c), .data_in(d), .data_out(q));`), the elaborator has no
//! SystemVerilog `module ringbuf` to inline. We synthesize one — an empty-body
//! module whose ports mirror the SystemC module's reflected `sc_in`/`sc_out`/
//! `sc_inout` members (from [`sc_module_ports`](super::options::sc_module_ports
//! )). Injecting this placeholder as an extra preprocessed source lets the
//! entire normal elaboration/port-connection/inlining pipeline run unchanged:
//! instance connections, hierarchical references (`u.data_out`), and
//! output-port propagation (`assign q = u.data_out`) all work exactly as for a
//! hand-written empty SystemVerilog module. The bridge then drives the SC leaf's
//! *port signals* directly (Import for `sc_out`, Export for `sc_in`), and the
//! already-synthesized output-port continuous assignments carry those values to
//! the parent actuals.
//!
//! Direction mapping: an SC `sc_in` port is *read* by SystemC (data flows
//! SV→SC, an Export row). An SC `sc_out` port is *driven* by SystemC (data
//! flows SC→SV, an Import row). `sc_inout` is genuinely bidirectional (Auto).

use super::options::ScDir;
use super::options::ScPort;

/// Render an empty-body SystemVerilog module that mirrors `ports`.
///
/// Ports are emitted in reflection order — inputs, outputs, then inouts
/// (connections are by-name so ordering is immaterial). Each maps SV `logic`
/// with the SC width: `sc_in<bool>` → `input logic clock`, `sc_uint<8>` →
/// `input logic [7:0] data_in`, `sc_out` → `output`, `sc_inout` → `inout`. A
/// 1-bit port is a plain `logic` (no range); a wider one gets `[N-1:0]`.
pub fn synthesize_placeholder(module: &str, ports: &[ScPort]) -> String {
    let range = |w: u32| -> String {
        if w <= 1 {
            "logic".to_string()
        } else {
            format!("logic [{}:0]", w - 1)
        }
    };

    let mut inherents: Vec<(&str, Vec<&ScPort>)> = Vec::new();
    let mut ins = Vec::new();
    let mut outs = Vec::new();
    let mut inouts = Vec::new();
    for p in ports {
        match p.dir {
            ScDir::Import => ins.push(p),
            ScDir::Export => outs.push(p),
            ScDir::Auto => inouts.push(p),
        }
    }
    inherents.push(("input", ins));
    inherents.push(("output", outs));
    inherents.push(("inout", inouts));

    let mut s = format!(
        "// Auto-generated placeholder for pure-SystemC module '{}' (Mode B).\n",
        module
    );
    s.push_str(&format!("module {}(", module));
    let mut decls: Vec<String> = Vec::new();
    for (dir, ps) in inherents {
        for p in ps {
            decls.push(format!("{} {} {}", dir, range(p.width), p.name));
        }
    }
    s.push_str(&decls.join(", "));
    s.push_str(");\nendmodule\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str, dir: ScDir, width: u32) -> ScPort {
        ScPort {
            name: name.to_string(),
            dir,
            width,
            align: ((width + 7) / 8).max(1),
            sc_type: String::new(),
        }
    }

    #[test]
    fn placeholder_inout_output_bits() {
        let ports = vec![
            port("clock", ScDir::Import, 1),
            port("data_in", ScDir::Import, 8),
            port("data_out", ScDir::Export, 8),
            port("bidir", ScDir::Auto, 16),
        ];
        let sv = synthesize_placeholder("ringbuf", &ports);
        assert!(sv.contains("module ringbuf("));
        assert!(sv.contains("input logic clock"));
        assert!(sv.contains("input logic [7:0] data_in"));
        assert!(sv.contains("output logic [7:0] data_out"));
        assert!(sv.contains("inout logic [15:0] bidir"));
        assert!(sv.trim_end().ends_with("endmodule"));
    }

    #[test]
    fn placeholder_no_ports() {
        let sv = synthesize_placeholder("const", &[]);
        assert!(sv.contains("module const("));
        assert!(sv.trim_end().ends_with("endmodule"));
    }

    #[test]
    fn placeholder_64_bit_align() {
        let ports = vec![port("wide", ScDir::Import, 64)];
        let sv = synthesize_placeholder("m", &ports);
        assert!(sv.contains("input logic [63:0] wide"));
    }
}