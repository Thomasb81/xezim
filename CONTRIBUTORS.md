# Contributors

Thank you to everyone who has improved xezim through pull requests:

- **Thomas Burg** — class-system and UVM fixes: static-property chains through
  object handles (§8.25), associative-array method dispatch and ref-writeback,
  `ClassName::static_prop` access, parser-gap self-tests, test-harness
  hardening, per-process bookkeeping for methods that park mid-body, the
  condition-waiter drain de-duplication, and the NBA-region lane in the
  `--max-time` hang report.
- **Vrajesh Prakhya** — real-number modelling: Verilog-AMS `wreal` nets
  resolved by summing, user-defined nettypes across the hierarchy and in
  packages (§6.6.7, §6.6.8), real-ness of members projected from call results,
  negative-test registrations, and the diagnosis that `cover property` sites
  were tallied as failing assertions.
- **Oscar Gustafsson** — expanded VPI functionality (`vpi_get_value`,
  `ObjectValType`), CI setup, and clippy cleanups.
- **Chen Ben Haroosh** — submodule-inline generate-for elaboration: genvar-
  dependent declarations and `parameter type` default resolution, plus the
  accompanying SystemVerilog compliance cases.
- **Jayaraman RP** — cross-platform installation scripts, including the macOS
  installer with UVM setup.
- **Ganesh T S** — wide-number parsing for `$sscanf`/`$fscanf`/`$value$plusargs`
  and the string conversion methods (arbitrary-precision decimal in the core),
  loop-variable scoping against same-named identifiers in child instances, and
  detailed self-checking bug reports for struct and class member access.
- **eenky** — declared defaults for parameters a class leaves out when it
  extends a parameterized class, and virtual-interface reads through
  class-handle chains (`cfg.vif.data`).
- **Francesco Urbani** — `+incdir+` directories in `-F` args files resolved
  against the args file's own directory.

New contributors are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md).
