//! Issue #233: zero-argument calls keep their call semantics when the empty
//! argument list is omitted. Enum calls have focused coverage elsewhere; this
//! module covers the other receiver and scope forms from the report.

use xezim::simulate;

#[test]
fn zero_argument_calls_work_without_parentheses() {
    let source = r#"
package call_pkg;
  function automatic int package_value(); return 7; endfunction
endpackage

function automatic int root_value(); return 42; endfunction

interface link_if;
  function int value(); return 3; endfunction
endinterface

class holder_c;
  int entries[$];
  function void fill(); entries.push_back(3); entries.push_back(4); endfunction
  function int from_root(); return root_value; endfunction
  function int take_front(); return entries.pop_front; endfunction
  function int take_back(); return entries.pop_back; endfunction
endclass

module top;
  import call_pkg::*;
  link_if link();
  holder_c holder = new;
  string word = "hello";
  int result;
  string changed;

  function automatic int wrapped_root(); return root_value; endfunction
  function automatic int wrapped_package(); return package_value; endfunction

  initial begin
    result = root_value;              $display("T root %0d", result);
    result = wrapped_root();          $display("T wrapped %0d", result);
    result = package_value;           $display("T imported %0d", result);
    result = call_pkg::package_value; $display("T scoped %0d", result);
    result = wrapped_package();       $display("T package_body %0d", result);
    result = word.len;                $display("T strlen %0d", result);
    changed = word.toupper;           $display("T upper %0s", changed);
    result = holder.from_root();      $display("T class_body %0d", result);
    holder.fill();
    result = holder.take_front();     $display("T front %0d %0d", result, holder.entries.size());
    result = holder.take_back();      $display("T back %0d %0d", result, holder.entries.size());
    result = link.value;              $display("T interface %0d", result);
  end
endmodule
"#;
    let sim = simulate(source, 1000).expect("simulate issue #233 matrix");
    let lines: Vec<&str> = sim
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .filter(|line| line.starts_with("T "))
        .collect();
    assert_eq!(
        lines,
        [
            "T root 42",
            "T wrapped 42",
            "T imported 7",
            "T scoped 7",
            "T package_body 7",
            "T strlen 5",
            "T upper HELLO",
            "T class_body 42",
            "T front 3 1",
            "T back 4 0",
            "T interface 3",
        ]
    );
}
