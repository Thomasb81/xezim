//! Integration-test group: gates.
//!
//! Every `tests/*.rs` used to build its own ~66 MB binary that statically
//! links the whole simulator; 374 of them cost 24 GB and dominated
//! `cargo test` wall-clock (the tests themselves run in milliseconds).
//! The cases now live one directory down and are included here as
//! modules, so this group links ONCE. Tests, names and assertions are
//! unchanged — only the link unit is. `#[path]` is required because a
//! crate root resolves `mod x;` beside itself, not into `tests/<group>/`.

#[path = "gates/assign_pattern_aggregate.rs"]
mod assign_pattern_aggregate;
#[path = "gates/drive_strength_pull.rs"]
mod drive_strength_pull;
#[path = "gates/dump_merged_sv.rs"]
mod dump_merged_sv;
#[path = "gates/specify_flags.rs"]
mod specify_flags;
#[path = "gates/tran_and_implicit_nets.rs"]
mod tran_and_implicit_nets;
#[path = "gates/udp_primitives.rs"]
mod udp_primitives;
#[path = "gates/vcd_lrm_compliance.rs"]
mod vcd_lrm_compliance;
