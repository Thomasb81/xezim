//! Integration-test group: hierarchy.
//!
//! Every `tests/*.rs` used to build its own ~66 MB binary that statically
//! links the whole simulator; 374 of them cost 24 GB and dominated
//! `cargo test` wall-clock (the tests themselves run in milliseconds).
//! The cases now live one directory down and are included here as
//! modules, so this group links ONCE. Tests, names and assertions are
//! unchanged — only the link unit is.
//!
//! The explicit module paths below are required: a crate root resolves a
//! plain `mod x;` beside itself, not into `tests/<group>/`. To add a test,
//! drop the file in this group's directory and add one entry here.

#[path = "hierarchy/array_of_module_instances.rs"]
mod array_of_module_instances;
#[path = "hierarchy/bind_directive_basic.rs"]
mod bind_directive_basic;
#[path = "hierarchy/bind_in_module.rs"]
mod bind_in_module;
#[path = "hierarchy/bind_upward_refs.rs"]
mod bind_upward_refs;
#[path = "hierarchy/c910_scoped_cont_assign.rs"]
mod c910_scoped_cont_assign;
#[path = "hierarchy/dump_merged_sv_top_pruning.rs"]
mod dump_merged_sv_top_pruning;
#[path = "hierarchy/final_blocks_hierarchy.rs"]
mod final_blocks_hierarchy;
#[path = "hierarchy/foreach_over_submodule_array.rs"]
mod foreach_over_submodule_array;
#[path = "hierarchy/generate_scope_names.rs"]
mod generate_scope_names;
#[path = "hierarchy/ifu_ibuf_32_instances_c910.rs"]
mod ifu_ibuf_32_instances_c910;
#[path = "hierarchy/implicit_net_gate_port.rs"]
mod implicit_net_gate_port;
#[path = "hierarchy/implicit_net_submodule.rs"]
mod implicit_net_submodule;
#[path = "hierarchy/ivtest_port_cluster.rs"]
mod ivtest_port_cluster;
#[path = "hierarchy/local_method_static_binding.rs"]
mod local_method_static_binding;
#[path = "hierarchy/module_timescale_cli.rs"]
mod module_timescale_cli;
#[path = "hierarchy/multi_instance_scope.rs"]
mod multi_instance_scope;
#[path = "hierarchy/multiple_top_modules.rs"]
mod multiple_top_modules;
#[path = "hierarchy/nested_cross_module_call.rs"]
mod nested_cross_module_call;
#[path = "hierarchy/nonansi_port_completion.rs"]
mod nonansi_port_completion;
#[path = "hierarchy/nonansi_ports_and_vif_accessors.rs"]
mod nonansi_ports_and_vif_accessors;
#[path = "hierarchy/null_ports_lib_defines_nowarn.rs"]
mod null_ports_lib_defines_nowarn;
#[path = "hierarchy/package_scoped_vars_and_methods.rs"]
mod package_scoped_vars_and_methods;
#[path = "hierarchy/percent_m_scope.rs"]
mod percent_m_scope;
#[path = "hierarchy/pkg_subroutines_and_unit_scope.rs"]
mod pkg_subroutines_and_unit_scope;
#[path = "hierarchy/repro_import.rs"]
mod repro_import;
#[path = "hierarchy/resource_pool_scope_lookup.rs"]
mod resource_pool_scope_lookup;
#[path = "hierarchy/same_name_port_hop.rs"]
mod same_name_port_hop;
#[path = "hierarchy/submodule_net_array_shapes.rs"]
mod submodule_net_array_shapes;
#[path = "hierarchy/submodule_two_dim_array_element.rs"]
mod submodule_two_dim_array_element;
#[path = "hierarchy/wildcard_import_shadow.rs"]
mod wildcard_import_shadow;
