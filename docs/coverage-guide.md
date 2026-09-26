# Coverage in xezim

xezim collects **functional coverage** (covergroups, IEEE 1800 clause 19) and
**assertion coverage** (`cover` statements, plus pass/fail counts of assertions,
clause 16) in every simulation, and writes the results to a JSON file when the run
ends. It does **not** collect code coverage: there is no line, statement, branch,
condition, expression, toggle or FSM coverage.

This guide covers what is supported, how to write coverage that xezim collects, and
how to read the results.

---

## What's supported

| Area | Supported |
|---|---|
| Where covergroups live | Any module (the top or an instantiated one), an interface, a package, file scope, and classes. See [Where a covergroup can be declared](#where-a-covergroup-can-be-declared) |
| Sampling | A sampling event (`covergroup cg @(posedge clk)`), explicit `sample()`, `with function sample(...)` arguments, constructor arguments (`covergroup cg (int lo, int hi)`) |
| Bins | Values and ranges (`{0, [2:5], [8:$]}`), automatic bins, array bins `name[]`, `wildcard` bins, transition bins `(1 => 2 => 3)` and `([0:1] => [2:3])`, `default`, `ignore_bins`, `illegal_bins` |
| Guards | `coverpoint x iff (cond)`, `cross a, b iff (cond)` |
| Crosses | Automatic cross bins (every combination of the coverpoints' bins), `bins name = binsof(cp) intersect {...}` |
| Options | `option.at_least`, `option.weight`, `option.auto_bin_max`, `type_option.merge_instances`, `type_option.weight` |
| Queries | `get_inst_coverage()` and `get_coverage()` on a covergroup, a coverpoint or a cross; `$get_coverage()` |
| Assertion coverage | Counts for `cover property`, `assert property`, `assume property` and the immediate `cover`, `assert` and `assume` |
| Results | The `xezim_cov.json` file, and `[COV]` summary lines with `--verbose` |

---

## Enabling coverage

Nothing needs enabling. Covergroups and assertions are always compiled and
evaluated, and a run that has at least one covergroup instance or assertion writes
`xezim_cov.json` to the current directory when it ends:

```bash
xezim alu_cov.sv          # also writes ./xezim_cov.json
```

- `XEZIM_COV_DB=<path>` writes the file somewhere else. `XEZIM_COV_DB=/dev/null`
  skips it.
- The coverage switches of other simulators' command lines (`+cover`,
  `+cover=<spec>`, `+fcover`, `-coverage`) are accepted and do nothing.

---

## Writing coverage

Give every coverpoint and cross a label (`cp_op : coverpoint op`). Queries and the
results file use the label as the name. An unlabeled coverpoint on a variable is
named after the variable (`coverpoint data` is `cg.data`); one on any other
expression gets the name `coverpoint#<n>`, `<n>` counting the covergroup's
coverpoints from 1, which a query cannot spell.

### A covergroup in a module, sampled on a clock

```systemverilog
module tb;
  logic       clk = 0;
  logic       valid;
  logic [1:0] op;
  logic [3:0] a;

  always #5 clk = ~clk;

  // Sampled on every rising clock edge.
  covergroup alu_cg @(posedge clk);
    cp_op : coverpoint op iff (valid) {   // skipped while valid is 0
      bins add       = {0};
      bins sub       = {1};
      bins logic_ops = {[2:3]};
    }
    cp_a : coverpoint a {
      bins zero = {0};
      bins low  = {[1:7]};
      bins high = {[8:15]};
    }
  endgroup

  alu_cg cg = new();

  initial begin
    valid = 1;
    op = 0; a = 0; @(negedge clk);
    op = 1; a = 3; @(negedge clk);
    op = 0; a = 5; @(negedge clk);
    valid = 0; op = 3; a = 9; @(negedge clk);   // op not sampled
    $display("cp_op  = %0.2f%%", cg.cp_op.get_inst_coverage());
    $display("cp_a   = %0.2f%%", cg.cp_a.get_inst_coverage());
    $display("alu_cg = %0.2f%%", cg.get_inst_coverage());
    $finish;
  end
endmodule
```

```text
$ xezim alu_cov.sv
cp_op  = 66.67%
cp_a   = 100.00%
alu_cg = 83.33%
Simulation finished at time 40 ($finish called)
```

`logic_ops` was never hit: the only `op = 3` came while `valid` was 0. So `cp_op` has
2 of its 3 bins, and the group is the average of its two coverpoints.

To print coverage when the run ends, however it ends (`$finish`, `$fatal` or
`--max-time`), put the queries in a `final` block:

```systemverilog
final $display("alu_cg: %0.2f%%", cg.get_inst_coverage());
```

### A covergroup in a class, sampled by `sample()`

A covergroup declared in a class is created in the class constructor. Its
coverpoints read the object's properties.

```systemverilog
class packet;
  rand bit [3:0] len;
  rand bit [1:0] kind;

  covergroup pkt_cg;
    cp_len : coverpoint len {
      bins short_p = {[0:3]};
      bins mid_p   = {[4:11]};
      bins long_p  = {[12:15]};
    }
    cp_kind : coverpoint kind;   // automatic bins: one per value, 4 bins
  endgroup

  function new();
    pkt_cg = new();              // a class covergroup is created in new()
  endfunction
endclass

module tb;
  initial begin
    packet p = new();
    void'(p.randomize() with { len == 2;  kind == 0; });  p.pkt_cg.sample();
    void'(p.randomize() with { len == 13; kind == 1; });  p.pkt_cg.sample();
    $display("cp_len  : %0.2f%%", p.pkt_cg.cp_len.get_inst_coverage());
    $display("cp_kind : %0.2f%%", p.pkt_cg.cp_kind.get_inst_coverage());
    $display("pkt_cg  : %0.2f%%", p.pkt_cg.get_inst_coverage());
    $display("overall : %0.2f%%", $get_coverage());
  end
endmodule
```

```text
$ xezim pkt_cov.sv
cp_len  : 66.67%
cp_kind : 50.00%
pkt_cg  : 58.33%
overall : 58.33%
Simulation finished at time 0
```

To pass the values in at the sampling call instead, declare the covergroup with a
sample function. This is the usual form for a monitor or subscriber:

```systemverilog
covergroup txn_cg with function sample (bit [3:0] addr, bit wr);
  cp_addr : coverpoint addr { bins lo = {[0:7]}; bins hi = {[8:15]}; }
  cp_wr   : coverpoint wr;
endgroup
...
txn_cg.sample(addr, wr);
```

### Crosses and `ignore_bins`

```systemverilog
module tb;
  bit [1:0] mode;
  bit [1:0] size;
  bit       en;

  covergroup cfg_cg;
    cp_mode : coverpoint mode {
      bins m[] = {[0:3]};            // one bin per value: m[0] .. m[3]
      ignore_bins reserved = {3};    // mode 3 is never used: drop m[3]
    }
    cp_size : coverpoint size {
      bins narrow = {0, 1};
      bins wide   = {2, 3};
    }
    mode_x_size : cross cp_mode, cp_size iff (en);   // 3 x 2 = 6 bins
  endgroup

  cfg_cg cg = new();

  initial begin
    en = 1;
    mode = 0; size = 0; cg.sample();
    mode = 1; size = 2; cg.sample();
    mode = 2; size = 3; cg.sample();
    mode = 3; size = 0; cg.sample();   // ignored mode: no cross bin
    en = 0;
    mode = 0; size = 3; cg.sample();   // cross not sampled (iff)
    $display("cp_mode     %0.2f", cg.cp_mode.get_inst_coverage());
    $display("cp_size     %0.2f", cg.cp_size.get_inst_coverage());
    $display("mode_x_size %0.2f", cg.mode_x_size.get_inst_coverage());
    $display("cfg_cg      %0.2f", cg.get_inst_coverage());
  end
endmodule
```

```text
$ xezim cross_cov.sv
cp_mode     100.00
cp_size     100.00
mode_x_size 50.00
cfg_cg      83.33
Simulation finished at time 0
```

The ignored value takes `m[3]` out of `cp_mode`, so the cross has 3 x 2 = 6 bins.
Three of them are hit: `(m[0], narrow)`, `(m[1], wide)` and `(m[2], wide)`. The last
sample is skipped by the cross's `iff` but still counts for the coverpoints.

To cover chosen parts of a cross instead of every combination, give the cross its
own bins. Coverage is then the fraction of those bins that were hit:

```systemverilog
axb : cross cp_a, cp_b {
  bins a_lo = binsof(cp_a) intersect {[0:1]};
  bins a_hi = binsof(cp_a) intersect {[2:3]};
}
```

### How coverage is computed

- **Explicit bins:** a coverpoint's coverage is the number of its `bins` that were hit
  divided by the number of `bins`. `ignore_bins` and `illegal_bins` values are removed
  from the other bins, and a bin left with no values is dropped. A `default` bin is
  not counted.
- **Automatic bins** (a coverpoint with no `bins`): one bin per value of the sampled
  expression's width, or `option.auto_bin_max` (default 64) equal ranges when there
  are more values than that. A 4-bit coverpoint has 16 bins; an `int` has 64.
- **Array bins** `name[]`: one bin per value, named `name[<value>]`.
- **Transition bins** `(1 => 2 => 3)`: hit when consecutive samples of the coverpoint
  match the sequence. Each step can be a value or a range.
- **Crosses:** every combination of the crossed coverpoints' bins. A crossed variable
  that has no coverpoint of its own gets automatic bins.
- **Covergroup:** the average of its coverpoints and crosses, weighted by each
  coverpoint's `option.weight` (default 1; a cross always weighs 1).

A hit on an `illegal_bins` value or transition is a run-time error: xezim prints
`** Error: Illegal bin hit at value 7: cg.cp_s.bad` with the time and scope, like
`$error`, and `--error-exit` counts it. The run carries on.

### Options and multiple instances

```systemverilog
covergroup opt_cg;
  option.at_least = 2;               // a bin needs 2 hits to count
  cp_x : coverpoint x { bins b0 = {0}; bins b1 = {1}; }
  cp_y : coverpoint y {
    option.weight   = 3;             // counts 3x in the group average
    option.at_least = 1;             // overrides the group setting
    bins b0 = {0}; bins b1 = {1};
  }
endgroup
```

After samples `(x, y)` = `(0, 0)`, `(0, 1)`, `(1, 1)`, `cp_x` has 50.00 (`b1` was hit
only once), `cp_y` 100.00, and `opt_cg` (50 + 3 x 100) / 4 = 87.50.

| Option | Where | Effect |
|---|---|---|
| `option.at_least = N` | covergroup, coverpoint or cross | A bin (explicit, automatic or cross) counts as covered once it has N hits (default 1). A coverpoint's or cross's setting overrides the group's |
| `option.weight = N` | coverpoint | Weight of the coverpoint in its covergroup's average (default 1) |
| `option.auto_bin_max = N` | covergroup or coverpoint | Maximum number of automatic bins (default 64) |
| `type_option.merge_instances = 1` | covergroup, or `cg_type::type_option.merge_instances = 1;` at run time | `get_coverage()` counts a bin as covered when any instance hit it. Default 0: the average of the instances' coverages |
| `type_option.weight = N` | covergroup, or `cg_type::type_option.weight = N;` | Weight of the covergroup type in `$get_coverage()` (default 1) |

Every `new()` creates a separate instance with its own hits. `get_inst_coverage()`
reports one instance; `get_coverage()` reports the covergroup type:

```systemverilog
covergroup val_cg with function sample (bit [3:0] v);
  cp : coverpoint v { bins lo = {[0:1]}; bins hi = {[14:15]}; }
endgroup

val_cg a = new();
val_cg b = new();

initial begin
  a.sample(0);    // a hits lo
  b.sample(15);   // b hits hi
  $display("a %0.2f  b %0.2f  type %0.2f", a.get_inst_coverage(), b.get_inst_coverage(), a.get_coverage());
  val_cg::type_option.merge_instances = 1;
  $display("merged type %0.2f", a.get_coverage());
end
```

```text
a 50.00  b 50.00  type 50.00
merged type 100.00
```

### Assertion coverage

Every `cover`, `assert` and `assume` is counted, with no extra code. A
`cover property` action block runs on each match:

```systemverilog
module tb;
  logic clk = 0, rst = 1, req = 0, gnt = 0;
  always #5 clk = ~clk;

  // Concurrent cover: a request answered by a grant on the next clock.
  c_req_gnt : cover property (@(posedge clk) disable iff (rst) req ##1 gnt)
    $display("%0t: request granted", $time);

  // Concurrent assert: never a grant without a request on the clock before.
  a_gnt_has_req : assert property (@(posedge clk) disable iff (rst) gnt |-> $past(req))
    else $error("grant without request");

  // Immediate cover, evaluated on every clock.
  always @(posedge clk) c_waiting : cover (req && !gnt);

  initial begin
    @(negedge clk) rst = 0;
    @(negedge clk) req = 1;
    @(negedge clk) begin req = 0; gnt = 1; end
    @(negedge clk) gnt = 0;
    @(negedge clk) req = 1;
    @(negedge clk) begin req = 0; gnt = 1; end
    @(negedge clk) gnt = 0;
    $finish;
  end
endmodule
```

```text
$ xezim handshake.sv
35: request granted
65: request granted
Simulation finished at time 70 ($finish called)
```

The counts are in the results file; see [Assertion entries](#assertion-entries).

---

## Collecting and reading results

### The results file

When the run ends, by `$finish`, `$fatal`, `--max-time` or running out of events,
xezim writes `xezim_cov.json` to the current directory, replacing any earlier one.

- It is written only when the design has at least one covergroup instance or one
  assertion that was evaluated. A run with neither leaves an old `xezim_cov.json`
  as it was.
- `XEZIM_COV_DB=<path>` writes it to `<path>` instead. The directory must exist.
  When the file can't be written, xezim prints
  `[COV] warning: could not write <path>: <reason>` and the run's result is
  unchanged.

The file for the [module example](#a-covergroup-in-a-module-sampled-on-a-clock):

```json
{
  "sim_time": 40,
  "assertion_sites": 0,
  "assertion_pass_total": 0,
  "assertion_fail_total": 0,
  "assertions": [
  ],
  "covergroups": [
    {"name": "alu_cg", "samples": 4, "coverpoints": {"cp_op": 2, "cp_a": 4}, "crosses": {}, "bins": {"cp_a.zero": 1, "cp_op.add": 2, "cp_op.sub": 1, "cp_a.low": 2, "cp_a.high": 1}}
  ]
}
```

| Field | Meaning |
|---|---|
| `sim_time` | The time the run ended: the same number as the closing `Simulation finished at time N` line |
| `assertion_sites` | Number of entries in `assertions` |
| `assertion_pass_total`, `assertion_fail_total` | The sums of the `pass` and `fail` fields of `assertions` |
| `assertions` | One entry per assertion or cover statement; see below |
| `covergroups` | One entry per covergroup instance, in creation order |
| `covergroups[].name` | The covergroup type: `alu_cg`, `packet::pkt_cg` for one declared in class `packet`, or `u0.v_cg` for one declared in the module instantiated as `u0` (the instance path below the top module). Instances of one type share the name |
| `covergroups[].samples` | How many times the instance was sampled (sampling events plus `sample()` calls) |
| `covergroups[].coverpoints` | Per coverpoint, the number of **distinct values** sampled (not bins) |
| `covergroups[].crosses` | Per cross, the number of distinct value combinations sampled |
| `covergroups[].bins` | Hit count per explicit bin, keyed `<coverpoint>.<bin>`, or `<coverpoint>.<bin>[<value>]` for array bins. Includes `default` bins |

Things the file does not hold:

- **Percentages.** Print them from the testbench with the
  [query functions](#query-functions), for example in a `final` block.
- **Bins that were never hit.** `alu_cg` above has no `cp_op.logic_ops` entry.
  Compare against the covergroup's source to find the holes.
- Automatic bins and the bins declared in a cross's body.

Key order in the objects can change from run to run. Compare files with
`jq -S . xezim_cov.json`, not with a plain `diff`.

#### Assertion entries

For the [assertion example](#assertion-coverage):

```json
  "assertions": [
    {"span_start": 95, "kind": "cover", "pass": 2, "fail": 0},
    {"span_start": 222, "kind": "assert", "pass": 6, "fail": 0},
    {"span_start": 372, "kind": "cover", "pass": 2, "fail": 5}
  ],
```

| Statement | `pass` | `fail` |
|---|---|---|
| `cover property` | Matches | Always 0 |
| `assert property`, `assume property` | Attempts that succeeded, including vacuous ones (`gnt` low, so `gnt \|-> ...` holds) | Attempts that failed |
| Immediate `cover`, `assert`, `assume` | Evaluations where the expression was true | Evaluations where it was false |

An immediate `cover`'s false evaluations therefore count in `assertion_fail_total`
too. In the example, all 5 fails are clock edges where `req && !gnt` was false; no
assertion failed.

An entry has no name, file or line. `span_start` is the position of the `cover` /
`assert` / `assume` keyword in its file after preprocessing, and the entries are
sorted by it. Preprocessing keeps line breaks, so for a single-file design the line
number is:

```bash
$ echo $(( $(xezim --preprocess handshake.sv | tail -n +2 | head -c 95 | wc -l) + 1 ))
6
```

All instances of a module share one entry: its counts are the sum over the
instances.

### The summary lines

`--verbose` (or `XEZIM_VERBOSE=1`) adds `[COV]` lines at the end of the run, on
stderr, together with the other engine lines:

```text
$ xezim --verbose alu_cov.sv 2>&1 | grep '^\[COV\]'
[COV] assertions: 0 sites (assert=0, assume=0, cover=0) — 0 passes, 0 fails
[COV] coverage: 1 covergroup instances, 4 samples, 6 unique coverpoint values, 0 unique cross tuples
[COV] wrote coverage DB to xezim_cov.json
```

The counts are totals over the whole design, with the same meanings as the file's
fields: "unique coverpoint values" adds up the file's `coverpoints` numbers (2 + 4).

### Query functions

All return a `real` percentage from 0.0 to 100.0; print it with `%f`, for example
`%0.2f`.

| Call | Returns |
|---|---|
| `cg.get_inst_coverage()` | Coverage of the instance `cg` |
| `cg.get_coverage()` | Coverage of the covergroup type: the average over its instances, or a union of their hits with `type_option.merge_instances = 1` |
| `cg.<cp>.get_inst_coverage()`, `cg.<cp>.get_coverage()` | The same for one coverpoint or cross `<cp>` |
| `$get_coverage()` | The average over every covergroup type, weighted by `type_option.weight` |
| `cg.sample()`, `cg.sample(args)` | Samples the instance now |

Call `get_coverage()` on an instance handle. The type-scoped forms
`cg_type::get_coverage()` and `cg_type::cp::get_coverage()` return 0.

### Several runs

There is no merge tool: every run writes its own file. Give each run its own path,
for example one per seed:

```systemverilog
module tb;
  bit [3:0] len;
  covergroup len_cg;
    cp_len : coverpoint len {
      bins short_p = {[0:3]};
      bins mid_p   = {[4:11]};
      bins long_p  = {[12:15]};
    }
  endgroup
  len_cg cg = new();
  initial begin
    repeat (2) begin
      len = $urandom_range(0, 15);
      cg.sample();
    end
    $display("len_cg = %0.2f%%", cg.get_inst_coverage());
  end
endmodule
```

```text
$ mkdir -p cov
$ for s in 1 2 3; do XEZIM_COV_DB=cov/seed$s.json xezim +seed=$s len_cov.sv; done
len_cg = 66.67%
Simulation finished at time 0
len_cg = 66.67%
Simulation finished at time 0
len_cg = 33.33%
Simulation finished at time 0
```

The files hold hit counts per bin, so the bins hit across all runs can be added up.
For example, with `jq`:

```text
$ jq -r '.covergroups[] | .name as $cg | .bins | to_entries[] | "\($cg).\(.key) \(.value)"' cov/seed*.json \
    | awk '{h[$1] += $2} END {for (b in h) print b, h[b]}' | sort
len_cg.cp_len.long_p 2
len_cg.cp_len.mid_p 4
```

`short_p` was not hit by any seed. The files don't record the bins that were never
hit, so a combined percentage can't be worked out from them alone.

---

## Limits and unsupported features

**Not supported at all:**

- Code coverage: line, statement, branch, condition, expression, toggle and FSM.
- Coverage databases in other formats (such as UCIS), text or HTML coverage
  reports, and merging results across runs.
- `cover sequence`: a parse error. Use `cover property` with the sequence.
- The coverage system functions `$coverage_control`, `$coverage_get`,
  `$coverage_get_max`, `$coverage_merge`, `$coverage_save`, `$set_coverage_db_name`
  and `$load_coverage_db`. They print `Warning: unknown system task '<name>' ignored`
  once, and the functions return 0.
- `coverage save` and `coverage report` in a `-do` script: ignored with a warning.

#### Where a covergroup can be declared

A covergroup can be declared in any module (the top or one instantiated below it,
and every top of a design with several), an interface, a package, at file scope,
and in a class wherever the class is declared.

A covergroup declared in a module or interface is a separate type in each instance
of that module: `get_coverage()` averages the covergroup's instances within that
module instance only, and `$get_coverage()` counts each module instance's type
once. A covergroup declared in a package is one type however many modules create
it.

**Covergroup features that are accepted but have no effect, or only a partial one:**

| Feature | What happens |
|---|---|
| `start()`, `stop()` | No effect: sampling continues after `stop()` |
| `option.per_instance`, `option.goal`, `option.name`, `option.comment` | Accepted, no effect. The results file always has one entry per instance, named after the type |
| `@(posedge clk iff cond)` as the sampling event | The `iff` is ignored and every edge samples. Put the condition on the coverpoints: `coverpoint x iff (cond)` |
| `bins name[N] = {...}` | Treated as `name[]`: one bin per value, not N bins |
| Transition sets and repetition: `(1, 5 => 3)`, `(3 [*2])`, `[->n]`, `[=n]` | Never hit |
| In a cross body: `ignore_bins`, `illegal_bins`, `binsof(cp.bin)`, `binsof(...) with (...)` | Ignored. Only `bins name = binsof(cp) intersect {...}` is used. To leave values out of a cross, put `ignore_bins` on the coverpoint, as in [the cross example](#crosses-and-ignore_bins) |
| `binsof(...) intersect {...} && binsof(...) ...` (or `\|\|`) | Only the first `binsof` term is used |
| Constructor arguments together with `with function sample` arguments | No bin is ever hit. Use one or the other |
| `get_coverage(covered, total)`, `get_inst_coverage(covered, total)` | The percentage is returned, but `covered` and `total` are not set |
| `cg_type::get_coverage()` | Returns 0. Call it on an instance: `cg.get_coverage()` |

---

## Quick reference

| Flag / variable | Effect |
|---|---|
| (none) | Covergroups and assertions are always collected. A run with any of them writes `./xezim_cov.json` |
| `XEZIM_COV_DB=<path>` | Write the results file to `<path>`; `/dev/null` to skip it |
| `--verbose`, `XEZIM_VERBOSE=1` | Print the `[COV]` summary lines on stderr, along with the other engine lines |
| `--preprocess <files>` | Print the preprocessed source, to map an assertion's `span_start` to a line |
| `+seed=<n>` | Seed the random generator, for reproducible random stimulus and coverage |
| `--error-exit` | Exit nonzero after any `$error`, including one from an assertion's action block, and after an `illegal_bins` hit |
| `+cover`, `+cover=<spec>`, `+fcover`, `-coverage` | Accepted for compatibility with other simulators; no effect |
| `-do "coverage save ..."`, `-do "coverage report ..."` | Ignored with a warning |
