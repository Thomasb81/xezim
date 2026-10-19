//! Compile the user's SystemC sources (+ shim + generated wrapper) into a
//! model/cosim shared object and load it.

use std::path::PathBuf;

use super::link::ScShimApi;
use super::options::{shim_include_dir, shim_source, ScOptions};
use super::wrapper::WrapperBundle;

/// Compile the user's SystemC sources together with the xezim shim into a
/// model/cosim shared object, and load it. Returns the API.
///
/// `bundle` carries the auto-derived wrapper (SC top: `xez_top.h`/`xez_top.cc`;
/// SV top: generated model TUs). When present its files are written into the
/// build dir and compiled in / placed on the include path.
pub fn build_and_load(opts: &ScOptions, bundle: &WrapperBundle) -> Result<ScShimApi, String> {
    let sc_home = std::env::var("SYSTEMC_HOME")
        .map_err(|_| "SYSTEMC_HOME is not set (required for SystemC co-simulation; \
                     point it at a SystemC 2.3+/3.x installation)".to_string())?;
    let inc = format!("{sc_home}/include");
    if !std::path::Path::new(&inc).join("systemc").exists() {
        return Err(format!("SYSTEMC_HOME={sc_home} has no include/systemc"));
    }

    if !opts.sources.is_empty() || opts.libs.is_empty() {
        let out_dir = std::env::temp_dir().join(format!(
            "xezim-sc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        ));
        std::fs::create_dir_all(&out_dir)
            .map_err(|e| format!("cannot create {out_dir:?}: {e}"))?;
        let so_path = out_dir.join("libxezim_sc_model.so");

        // Materialise the auto-derived wrapper into the build dir.
        let mut generated_tus: Vec<PathBuf> = Vec::new();
        if !bundle.sc_top_header.is_empty() {
            std::fs::write(out_dir.join("xez_top.h"), &bundle.sc_top_header)
                .map_err(|e| format!("cannot write xez_top.h: {e}"))?;
        }
        if !bundle.sc_top_impl.is_empty() {
            let p = out_dir.join("xez_top.cc");
            std::fs::write(&p, &bundle.sc_top_impl)
                .map_err(|e| format!("cannot write xez_top.cc: {e}"))?;
            generated_tus.push(p);
        }
        for (i, src) in bundle.sv_top_sources.iter().enumerate() {
            let p = out_dir.join(format!("xez_model_{i}.cc"));
            std::fs::write(&p, src).map_err(|e| format!("cannot write generated model TU: {e}"))?;
            generated_tus.push(p);
        }

        let mut cmd = std::process::Command::new("g++");
        cmd.arg("-shared")
            .arg("-fPIC")
            .arg("-std=c++17")
            .arg("-O1")
            .arg("-I")
            .arg(&inc)
            .arg("-I")
            .arg(shim_include_dir());
        for d in &opts.include_dirs {
            cmd.arg("-I").arg(d);
        }
        if !bundle.sc_top_header.is_empty() {
            // So the user's `#include "xez_top.h"` resolves.
            cmd.arg("-I").arg(&out_dir);
        }
        if bundle.sv_top_sources.is_empty() {
            // In SV-master auto-derive mode the user source (SC_MODULE
            // definition) is `#include`d into the generated model TU, so it
            // must NOT also be a standalone TU (duplicate ODR symbols). In
            // SC-master mode the user source is the testbench.
            for s in &opts.sources {
                cmd.arg(s);
            }
        } else {
            // SV-master mode: the generated model TU `#include`s each user
            // source by path, so its parent directory must be on the include
            // path for relative --sc-source paths to resolve from the temp
            // build dir. Resolve against the user's cwd (the process dir).
            for s in &opts.sources {
                let abs = std::fs::canonicalize(s)
                    .unwrap_or_else(|_| std::path::PathBuf::from(s));
                if let Some(dir) = abs.parent() {
                    cmd.arg("-I").arg(dir);
                }
            }
        }
        for t in &generated_tus {
            cmd.arg(t);
        }
        for f in &opts.cxxflags {
            cmd.arg(f);
        }
        cmd.arg(shim_source());
        let libdir = format!("{sc_home}/lib");
        cmd.arg("-L").arg(&libdir).arg("-lsystemc");
        // Also probe lib64 (some distributions install there).
        if std::path::Path::new(&format!("{sc_home}/lib64")).exists() {
            cmd.arg("-L").arg(format!("{sc_home}/lib64"));
        }
        cmd.arg(format!("-Wl,-rpath,{libdir}"))
            .arg("-o")
            .arg(&so_path);

        let out = cmd
            .output()
            .map_err(|e| format!("cannot run g++ for the SystemC model: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "SystemC model compilation failed:\n{}{}",
                String::from_utf8_lossy(&out.stderr),
                String::from_utf8_lossy(&out.stdout)
            ));
        }
        return ScShimApi::load(so_path.to_string_lossy().as_ref());
    }

    // Prebuilt model only.
    ScShimApi::load(&opts.libs[0])
}