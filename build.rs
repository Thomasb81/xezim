#[cfg(feature = "tbb")]
extern crate cc;

fn main() {
    #[cfg(feature = "tbb")]
    {
        cc::Build::new()
            .cpp(true)
            .file("src/tbb_shim.cpp")
            .flag_if_supported("-std=c++17")
            .flag_if_supported("-O2")
            .compile("xezim_tbb_shim");
        // Link against the system Intel TBB.
        println!("cargo:rustc-link-lib=tbb");
        println!("cargo:rerun-if-changed=src/tbb_shim.cpp");
    }
}
