//! IEEE 1800-2023 clause 26: packages.

use crate::harness::{Order, check};

// §26.3: package var local shadow
#[test]
fn c26_3_package_var_local_shadow() {
    check(
        "26.3_package_var_local_shadow",
        include_str!("sv/26.3_package_var_local_shadow.sv"),
        "rpl",
        Order::Exact,
        &["T|r1|local=99 pkg=1"],
        &[],
    );
}

// §26.3: Variables with the same name in two packages, or in a package and an importing module
#[test]
fn c26_3_package_var_name_aliasing() {
    check(
        "26.3_package_var_name_aliasing",
        include_str!("sv/26.3_package_var_name_aliasing.sv"),
        "rpv",
        Order::Exact,
        &["T|r1|pa=1 pc=50", "T|r2|pa=2 pc=50", "T|r3|pa=2 pc=7"],
        &[],
    );
}

// §26.4: package import rules
#[test]
fn c26_4_package_import_rules() {
    check(
        "c26_more",
        include_str!("sv/c26_more.sv"),
        "c26x",
        Order::Exact,
        &["T|26.3e|99 1", "T|26.4|10 3", "T|26.3f|32"],
        &[],
    );
}
